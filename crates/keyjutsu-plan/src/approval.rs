//! Approval, and the immutable snapshot it produces.
//!
//! The operator edits a draft plan. Approving a step records the step's
//! current hash. If the step, or anything before it, changes afterwards, its
//! hash changes and the approval no longer matches: the step is shown as
//! invalidated, never silently kept.
//!
//! Critical steps cannot be approved wholesale. Each needs its own typed
//! confirmation phrase: "approve everything" skips them and says so.
//!
//! Sealing turns a fully approved draft into an [`ApprovedSnapshot`]. A
//! snapshot has no mutating methods. Loading one from JSON re-checks every
//! hash, so a snapshot edited on disk is refused rather than executed.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::graph::PlanGraph;
use crate::hash::{EnvironmentFingerprint, hash_value, step_hashes};
use crate::model::{Plan, RiskLevel, Step};
use crate::parse::{PlanError, ValidPlan};

const SNAPSHOT_KIND: &str = "keyjutsu.snapshot/1";

/// One step's approval: which version of the step the operator approved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "plan/")]
pub struct Approval {
    pub step: String,
    pub step_hash: String,
    /// RFC 3339, supplied by the caller: this crate reads no clock.
    pub at: String,
    /// The phrase typed for a critical step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub confirmation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(tag = "status", rename_all = "snake_case")]
#[ts(export, export_to = "plan/")]
pub enum StepApproval {
    Approved,
    NotApproved,
    /// Approved once, but the step or something before it has changed since.
    Invalidated {
        approved_hash: String,
        current_hash: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApprovalError {
    #[error("there is no step `{0}`")]
    UnknownStep(String),
    #[error("step `{step}` is critical and needs the typed confirmation `{phrase}`")]
    ConfirmationRequired { step: String, phrase: String },
    #[error("the confirmation for step `{step}` does not match `{phrase}`")]
    ConfirmationMismatch { step: String, phrase: String },
}

/// Whether a step needs its own typed confirmation. Today that is the agent's
/// proposed risk or KeyJutsu's own assessment saying critical; KeyJutsu's
/// independent risk rules arrive with validation, so an agent
/// that under-states risk is not yet caught here.
pub fn is_critical(plan: &Plan, step: &Step) -> bool {
    let proposed = step.proposed_risk.as_ref().is_some_and(|r| r.level == RiskLevel::Critical);
    let assessed = plan
        .keyjutsu
        .as_ref()
        .and_then(|k| k.steps.get(&step.id))
        .and_then(|s| s.assessed_risk)
        .is_some_and(|r| r == RiskLevel::Critical);
    proposed || assessed
}

/// The phrase an operator types to approve a critical step: its title in
/// capitals, letters and digits only, so it names the action rather than
/// being a generic "yes".
pub fn confirmation_phrase(step: &Step) -> String {
    step.title
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_uppercase().next().unwrap_or(c) } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The operator's approvals for one draft.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalBook {
    approvals: BTreeMap<String, Approval>,
}

impl ApprovalBook {
    pub fn new() -> Self {
        Self::default()
    }

    /// Approve one step as it is now.
    pub fn approve(
        &mut self,
        plan: &ValidPlan,
        step: &str,
        at: &str,
        confirmation: Option<&str>,
    ) -> Result<(), ApprovalError> {
        let s = plan.plan().step(step).ok_or_else(|| ApprovalError::UnknownStep(step.to_owned()))?;
        let mut confirmed = None;
        if is_critical(plan.plan(), s) {
            let phrase = confirmation_phrase(s);
            match confirmation.map(str::trim) {
                None => return Err(ApprovalError::ConfirmationRequired { step: step.to_owned(), phrase }),
                Some(typed) if typed != phrase => {
                    return Err(ApprovalError::ConfirmationMismatch { step: step.to_owned(), phrase });
                }
                Some(typed) => confirmed = Some(typed.to_owned()),
            }
        }
        let hashes = step_hashes(plan.plan(), plan.graph());
        let step_hash = hashes.get(step).cloned().unwrap_or_default();
        self.approvals.insert(
            step.to_owned(),
            Approval { step: step.to_owned(), step_hash, at: at.to_owned(), confirmation: confirmed },
        );
        Ok(())
    }

    /// Approve every step that does not need its own confirmation. Returns
    /// the critical steps it left alone, which the caller must show.
    pub fn approve_all_except_critical(&mut self, plan: &ValidPlan, at: &str) -> Vec<String> {
        let mut skipped = Vec::new();
        for id in plan.graph().topological_order() {
            match self.approve(plan, id, at, None) {
                Ok(()) => {}
                Err(_) => skipped.push(id.to_owned()),
            }
        }
        skipped
    }

    /// Withdraw an approval.
    pub fn revoke(&mut self, step: &str) {
        self.approvals.remove(step);
    }

    /// Where each step stands against the plan as it is now.
    pub fn status(&self, plan: &Plan, graph: &PlanGraph) -> BTreeMap<String, StepApproval> {
        let hashes = step_hashes(plan, graph);
        hashes
            .into_iter()
            .map(|(id, current)| {
                let status = match self.approvals.get(&id) {
                    None => StepApproval::NotApproved,
                    Some(a) if a.step_hash == current => StepApproval::Approved,
                    Some(a) => StepApproval::Invalidated {
                        approved_hash: a.step_hash.clone(),
                        current_hash: current,
                    },
                };
                (id, status)
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SealError {
    #[error("{} not approved as they stand: {}", crate::count(.0.len(), "step is", "steps are"), .0.join(", "))]
    NotApproved(Vec<String>),
    /// Validation found these steps not READY: a plan must not arm with
    /// a step that is blocked, invalid, awaiting review or awaiting revalidation).
    #[error("{} not ready: {}", crate::count(.0.len(), "step is", "steps are"), .0.join(", "))]
    NotReady(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SnapshotError {
    #[error("this is not a KeyJutsu snapshot: {0}")]
    NotASnapshot(String),
    #[error("snapshot format `{found}` is not supported; this KeyJutsu reads `{SNAPSHOT_KIND}`")]
    UnsupportedFormat { found: String },
    #[error("the plan inside the snapshot is invalid: {0}")]
    Plan(PlanError),
    /// The recorded hashes do not match the content: the snapshot was edited.
    #[error("the snapshot has been altered: {0}")]
    Tampered(String),
}

/// An approved plan, frozen. Construct it with [`seal`] or [`ApprovedSnapshot::from_json`];
/// there is no way to change one.
#[derive(Debug, Clone, PartialEq)]
pub struct ApprovedSnapshot {
    plan: ValidPlan,
    step_hashes: BTreeMap<String, String>,
    approvals: Vec<Approval>,
    fingerprint: Option<EnvironmentFingerprint>,
    sealed_at: String,
    snapshot_hash: String,
}

/// Seal a fully approved draft. Every step must be approved at its current
/// hash. If the draft carries validation results (KeyJutsu's own state
/// section), every step must be READY, and the results are sealed with it:
/// they are the evidence the approval was given on, and they are where
/// KeyJutsu's own risk assessment lives.
pub fn seal(
    draft: &ValidPlan,
    book: &ApprovalBook,
    fingerprint: Option<EnvironmentFingerprint>,
    at: &str,
) -> Result<ApprovedSnapshot, SealError> {
    let status = book.status(draft.plan(), draft.graph());
    let missing: Vec<String> = draft
        .graph()
        .topological_order()
        .filter(|id| status.get(*id) != Some(&StepApproval::Approved))
        .map(str::to_owned)
        .collect();
    if !missing.is_empty() {
        return Err(SealError::NotApproved(missing));
    }
    if let Some(state) = &draft.plan().keyjutsu {
        let not_ready: Vec<String> = draft
            .graph()
            .topological_order()
            .filter(|id| state.steps.get(*id).map(|s| s.readiness) != Some(crate::model::Readiness::Ready))
            .map(str::to_owned)
            .collect();
        if !not_ready.is_empty() {
            return Err(SealError::NotReady(not_ready));
        }
    }
    let mut plan = draft.plan().clone();
    if let Some(state) = &mut plan.keyjutsu {
        // Hashes and approvals live in the snapshot itself, not in the plan.
        state.snapshot_hash = None;
        for s in state.steps.values_mut() {
            s.step_hash = None;
            s.approved = None;
        }
    }
    // Re-checking cannot fail: the draft already passed every gate.
    let plan = ValidPlan::revalidate(plan, false).map_err(|_| SealError::NotApproved(Vec::new()))?;
    let step_hashes = step_hashes(plan.plan(), plan.graph());
    let approvals: Vec<Approval> =
        plan.graph().topological_order().filter_map(|id| book.approvals.get(id).cloned()).collect();
    let snapshot_hash = snapshot_hash(plan.plan(), &step_hashes, &approvals, fingerprint.as_ref(), at);
    Ok(ApprovedSnapshot {
        plan,
        step_hashes,
        approvals,
        fingerprint,
        sealed_at: at.to_owned(),
        snapshot_hash,
    })
}

fn snapshot_hash(
    plan: &Plan,
    step_hashes: &BTreeMap<String, String>,
    approvals: &[Approval],
    fingerprint: Option<&EnvironmentFingerprint>,
    at: &str,
) -> String {
    hash_value(&json!({
        "kind": SNAPSHOT_KIND,
        "plan": plan,
        "step_hashes": step_hashes,
        "approvals": approvals,
        "fingerprint": fingerprint.map(EnvironmentFingerprint::hash),
        "sealed_at": at,
    }))
}

/// The stored form. Field order is fixed by the struct, so the file is stable.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotFile {
    kind: String,
    snapshot_hash: String,
    sealed_at: String,
    step_hashes: BTreeMap<String, String>,
    approvals: Vec<Approval>,
    fingerprint: Option<EnvironmentFingerprint>,
    plan: Value,
}

impl ApprovedSnapshot {
    pub fn plan(&self) -> &Plan {
        self.plan.plan()
    }

    pub fn graph(&self) -> &PlanGraph {
        self.plan.graph()
    }

    pub fn step_hashes(&self) -> &BTreeMap<String, String> {
        &self.step_hashes
    }

    pub fn approvals(&self) -> &[Approval] {
        &self.approvals
    }

    pub fn fingerprint(&self) -> Option<&EnvironmentFingerprint> {
        self.fingerprint.as_ref()
    }

    pub fn sealed_at(&self) -> &str {
        &self.sealed_at
    }

    /// The answer to "what exact plan was executed?"
    pub fn snapshot_hash(&self) -> &str {
        &self.snapshot_hash
    }

    pub fn to_json(&self) -> String {
        let file = SnapshotFile {
            kind: SNAPSHOT_KIND.into(),
            snapshot_hash: self.snapshot_hash.clone(),
            sealed_at: self.sealed_at.clone(),
            step_hashes: self.step_hashes.clone(),
            approvals: self.approvals.clone(),
            fingerprint: self.fingerprint.clone(),
            plan: serde_json::to_value(self.plan.plan()).unwrap_or(Value::Null),
        };
        serde_json::to_string_pretty(&file).unwrap_or_default()
    }

    /// Load a stored snapshot, re-checking the plan through every gate and
    /// every hash against the content. Anything that does not match is
    /// refused: a snapshot is either exactly what was approved or unusable.
    pub fn from_json(text: &str) -> Result<Self, SnapshotError> {
        let value: Value =
            serde_json::from_str(text).map_err(|e| SnapshotError::NotASnapshot(e.to_string()))?;
        match value.get("kind").and_then(Value::as_str) {
            Some(SNAPSHOT_KIND) => {}
            Some(other) => return Err(SnapshotError::UnsupportedFormat { found: other.to_owned() }),
            None => return Err(SnapshotError::NotASnapshot("it has no `kind`".into())),
        }
        let file: SnapshotFile =
            serde_json::from_value(value).map_err(|e| SnapshotError::NotASnapshot(e.to_string()))?;
        let plan = crate::parse::parse_plan(&file.plan.to_string()).map_err(SnapshotError::Plan)?;

        let recomputed = step_hashes(plan.plan(), plan.graph());
        if recomputed != file.step_hashes {
            let changed: Vec<&String> = recomputed
                .keys()
                .chain(file.step_hashes.keys())
                .filter(|k| recomputed.get(*k) != file.step_hashes.get(*k))
                .collect();
            return Err(SnapshotError::Tampered(format!(
                "step hashes do not match the steps (changed: {})",
                changed.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
            )));
        }
        for id in plan.graph().topological_order() {
            let approved = file.approvals.iter().find(|a| a.step == id);
            match approved {
                None => return Err(SnapshotError::Tampered(format!("step `{id}` has no approval"))),
                Some(a) if Some(&a.step_hash) != recomputed.get(id) => {
                    return Err(SnapshotError::Tampered(format!(
                        "the approval of `{id}` is for a different step"
                    )));
                }
                Some(a) => {
                    let step = plan.plan().step(id).ok_or_else(|| SnapshotError::Tampered(id.to_owned()))?;
                    if is_critical(plan.plan(), step)
                        && a.confirmation.as_deref() != Some(&confirmation_phrase(step))
                    {
                        return Err(SnapshotError::Tampered(format!(
                            "critical step `{id}` lacks its typed confirmation"
                        )));
                    }
                }
            }
        }
        if file.approvals.len() != recomputed.len() {
            return Err(SnapshotError::Tampered("there are approvals for steps that do not exist".into()));
        }
        let expected = snapshot_hash(
            plan.plan(),
            &recomputed,
            &file.approvals,
            file.fingerprint.as_ref(),
            &file.sealed_at,
        );
        if expected != file.snapshot_hash {
            return Err(SnapshotError::Tampered("the snapshot hash does not match its content".into()));
        }
        Ok(Self {
            plan,
            step_hashes: recomputed,
            approvals: file.approvals,
            fingerprint: file.fingerprint,
            sealed_at: file.sealed_at,
            snapshot_hash: file.snapshot_hash,
        })
    }
}
