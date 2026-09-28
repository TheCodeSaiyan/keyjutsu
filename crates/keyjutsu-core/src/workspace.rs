//! The plan workspace: the draft the operator inspects and edits
//! before anything runs.
//!
//! Every change goes through here and is re-read as a whole plan, so an edit
//! the schema or the structure check would refuse never becomes the draft.
//! A change keeps KeyJutsu's validation only for the steps it cannot have
//! affected: the changed step and everything that follows it in the graph go
//! back to "needs validation": revalidation follows the dependencies.
//! Nothing here runs a plan; approval seals a snapshot that the executor runs.

use std::collections::BTreeMap;

use keyjutsu_agent::context::PreparedContext;
use keyjutsu_agent::review::{Review, Severity};
use keyjutsu_agent::session::record_review;
use keyjutsu_agent::{AgentError, AgentHandle, Agents, Runner, StepRevision};
use keyjutsu_plan::approval::{
    ApprovalBook, ApprovalError, ApprovedSnapshot, confirmation_phrase, is_critical, seal,
};
use keyjutsu_plan::diff::{PlanDiff, diff};
use keyjutsu_plan::hash::EnvironmentFingerprint;
use keyjutsu_plan::model::{
    Actor, ActorKind, KeyJutsuState, Plan, Privilege, ProvenanceAction, ProvenanceEvent, Readiness,
    RiskLevel, Step, StepKind,
};
use keyjutsu_plan::{PlanError, ValidPlan, parse_plan};
use keyjutsu_validation::{Options, validate};
use serde::Serialize;

/// Something said about the plan, shown beside it. The plan is authoritative;
/// these explain how it got that way. Chat assists; it never decides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "workspace/")]
pub struct Note {
    /// "You", the agent's name, or "KeyJutsu".
    pub who: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub step: Option<String>,
    /// A reviewer's finding, rather than something the author said.
    pub review: bool,
    /// A reviewer's finding the operator decided needs no change.
    #[serde(default)]
    pub dismissed: bool,
    pub at: String,
}

/// One step as the plan list shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "workspace/")]
pub struct StepSummary {
    pub id: String,
    pub number: u32,
    pub title: String,
    /// Shell, privilege and reversibility, for the line under the title.
    pub detail: String,
    /// `None` until validated, and again after a change that affects it.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub readiness: Option<Readiness>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub risk: Option<RiskLevel>,
    pub critical: bool,
    /// The phrase approval needs, for a critical step.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub confirmation_phrase: Option<String>,
    /// Review findings about this step that are warnings or worse.
    pub concerns: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "workspace/")]
pub struct Overall {
    pub total: u32,
    pub ready: u32,
    pub needs_review: u32,
    pub blocked: u32,
    pub invalid: u32,
    pub unvalidated: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub highest_risk: Option<RiskLevel>,
    pub needs_elevation: bool,
    /// Every reversible step says how it would be undone.
    pub recovery_prepared: bool,
    /// Why approval is not possible yet; empty when it is.
    pub blocking: Vec<String>,
}

/// Everything the workspace shows, in one message.
#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "workspace/")]
pub struct WorkspaceView {
    pub task: String,
    /// The whole draft, for the step detail and editor.
    pub plan: Plan,
    /// Steps in execution order.
    pub steps: Vec<StepSummary>,
    pub overall: Overall,
    pub notes: Vec<Note>,
    /// What the last change did, including which steps it sent back to
    /// validation.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub last_change: Option<PlanDiff>,
    /// Things validation itself could not do.
    pub problems: Vec<String>,
    /// What needs the operator, with the choices that answer it (ADR 0020).
    pub asks: Vec<crate::asks::Ask>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorkspaceError {
    #[error("{0}")]
    Plan(String),
    #[error("there is no step `{0}`")]
    UnknownStep(String),
    #[error("a step `{0}` already exists")]
    DuplicateStep(String),
    #[error("{0}")]
    Agent(String),
    #[error("{0}")]
    Approval(String),
}

impl From<PlanError> for WorkspaceError {
    fn from(e: PlanError) -> Self {
        Self::Plan(e.explain())
    }
}

impl From<AgentError> for WorkspaceError {
    fn from(e: AgentError) -> Self {
        Self::Agent(e.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct Workspace {
    task: String,
    draft: ValidPlan,
    notes: Vec<Note>,
    review: Option<Review>,
    last_change: Option<PlanDiff>,
    problems: Vec<String>,
}

/// The plan allows 4,000 characters in a provenance note. What goes in one
/// is two texts the operator or an agent wrote, a concern and a reason or a
/// question and an answer, each of which may be nearly that long, so each
/// keeps up to 1,500.
fn clip(text: &str) -> String {
    if text.chars().count() <= 1500 {
        text.to_owned()
    } else {
        format!("{}…", text.chars().take(1499).collect::<String>())
    }
}

/// The record that the operator answered question `q`.
fn answered(q: &keyjutsu_plan::model::Question, answer: &str, at: &str) -> ProvenanceEvent {
    ProvenanceEvent {
        step: q.step.clone(),
        actor: operator(),
        action: ProvenanceAction::Answered,
        at: at.into(),
        note: Some(format!("asked: {} — answered: {}", clip(&q.text), clip(answer))),
    }
}

fn operator() -> Actor {
    Actor { kind: ActorKind::Operator, agent: None }
}

fn agent_name(plan: &Plan) -> String {
    serde_json::to_value(plan.agent.name)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "agent".into())
}

impl Workspace {
    pub fn new(task: impl Into<String>, draft: ValidPlan) -> Self {
        Self {
            task: task.into(),
            draft,
            notes: Vec::new(),
            review: None,
            last_change: None,
            problems: Vec::new(),
        }
    }

    /// A plan from a file: a proposal or a stored plan with KeyJutsu's state.
    /// Where the desktop app saves plans: beside KeyJutsu's other data.
    pub fn plans_dir() -> std::path::PathBuf {
        crate::store::default_root().with_file_name("plans")
    }

    pub fn open(text: &str) -> Result<Self, WorkspaceError> {
        let draft = parse_plan(text)?;
        let task = draft.plan().title.clone().unwrap_or_else(|| draft.plan().task_id.clone());
        Ok(Self::new(task, draft))
    }

    pub fn plan(&self) -> &Plan {
        self.draft.plan()
    }

    pub fn draft(&self) -> &ValidPlan {
        &self.draft
    }

    /// Save the draft as it stands, every step's validation findings with
    /// it, to a new dated file in `dir`, and say where. Something to keep, or
    /// to hand to whoever is helping with a plan that will not go: it opens
    /// again with Open plan file…, or `keyjutsu plan validate FILE`. It holds
    /// the plan's commands and paths, as the agent wrote them; nothing the
    /// operator typed as a credential is ever in a plan.
    pub fn save_draft(&self, dir: &std::path::Path) -> Result<std::path::PathBuf, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
        let stamp = crate::fingerprint::now_rfc3339().replace(':', "");
        let file = dir.join(format!("{}-{stamp}.json", self.plan().plan_id));
        let text = serde_json::to_string_pretty(self.plan()).map_err(|e| e.to_string())?;
        std::fs::write(&file, text).map_err(|e| format!("could not write {}: {e}", file.display()))?;
        Ok(file)
    }

    pub fn task(&self) -> &str {
        &self.task
    }

    /// Accept KeyJutsu's own risk rating for step `id`, where the agent rated
    /// it lower: the step's proposed risk becomes KeyJutsu's, which sends it
    /// back to validation like any edit.
    pub fn use_keyjutsu_rating(&mut self, id: &str, at: &str) -> Result<PlanDiff, WorkspaceError> {
        let state = self.draft.plan().keyjutsu.as_ref().and_then(|k| k.steps.get(id));
        let level = state.and_then(|s| s.assessed_risk).ok_or_else(|| {
            WorkspaceError::Plan(format!("`{id}` has no rating from KeyJutsu yet: validate first"))
        })?;
        let reasons = state.map(|s| s.risk_reasons.join("; ")).unwrap_or_default();
        let mut step =
            self.draft.plan().step(id).cloned().ok_or_else(|| WorkspaceError::UnknownStep(id.into()))?;
        step.proposed_risk = Some(keyjutsu_plan::model::ProposedRisk {
            level,
            rationale: format!("KeyJutsu's rating, accepted by the operator: {reasons}"),
        });
        self.replace_step(step, at)
    }

    /// Mark reviewer's concern `index` as needing no change, for `reason`,
    /// and record that in the plan's provenance, where Save plan and the
    /// history keep it. Nothing in the plan changes, so nothing goes back to
    /// validation.
    pub fn dismiss(&mut self, index: usize, reason: &str, at: &str) -> Result<(), WorkspaceError> {
        let note = self
            .notes
            .get(index)
            .filter(|n| n.review && !n.dismissed)
            .cloned()
            .ok_or_else(|| WorkspaceError::Plan("there is no such concern to dismiss".into()))?;
        let mut plan = self.draft.plan().clone();
        let state = plan.keyjutsu.get_or_insert_with(|| KeyJutsuState {
            revision: None,
            steps: BTreeMap::new(),
            snapshot_hash: None,
            provenance: Vec::new(),
        });
        let reason = reason.trim();
        state.provenance.push(ProvenanceEvent {
            step: note.step.clone(),
            actor: operator(),
            action: ProvenanceAction::Dismissed,
            at: at.into(),
            note: Some(format!(
                "{}'s concern: {}{}",
                note.who,
                clip(&note.text),
                if reason.is_empty() {
                    String::new()
                } else {
                    format!(" — dismissed because: {}", clip(reason))
                }
            )),
        });
        self.draft = ValidPlan::revalidate(plan, false)?;
        self.notes[index].dismissed = true;
        Ok(())
    }

    pub fn note(&mut self, who: &str, text: &str, step: Option<&str>, at: &str) {
        self.notes.push(Note {
            who: who.into(),
            text: text.into(),
            step: step.map(str::to_owned),
            review: false,
            dismissed: false,
            at: at.into(),
        });
    }

    /// Replace the draft with `next`, keeping validation only where the
    /// change cannot reach, and recording who changed which steps.
    fn adopt(
        &mut self,
        mut next: Plan,
        actor: Option<(Actor, ProvenanceAction, Option<String>)>,
        at: &str,
    ) -> Result<PlanDiff, WorkspaceError> {
        // Check the new plan's structure before comparing against it.
        let checked = ValidPlan::revalidate(Plan { keyjutsu: None, ..next.clone() }, false)?;
        let change = diff(self.draft.plan(), &next, checked.graph());
        let mut state = self.draft.plan().keyjutsu.clone().unwrap_or(KeyJutsuState {
            revision: None,
            steps: BTreeMap::new(),
            snapshot_hash: None,
            provenance: Vec::new(),
        });
        if let Some(incoming) = next.keyjutsu.take() {
            // An agent revision brings its own provenance and no validation.
            state.provenance = incoming.provenance;
            state.steps.retain(|id, _| incoming.steps.contains_key(id) || !change.affected.contains(id));
        }
        state.steps.retain(|id, _| next.step(id).is_some() && !change.affected.contains(id));
        state.snapshot_hash = None;
        if let Some((actor, action, note)) = actor {
            let steps: Vec<Option<String>> = if change.affected.is_empty() && change.changed.is_empty() {
                vec![None]
            } else {
                change
                    .added
                    .iter()
                    .chain(change.changed.iter().map(|c| &c.step))
                    .map(|s| Some(s.clone()))
                    .collect()
            };
            for step in steps {
                state.provenance.push(ProvenanceEvent {
                    step,
                    actor: actor.clone(),
                    action,
                    at: at.to_owned(),
                    note: note.clone(),
                });
            }
        }
        next.keyjutsu = Some(state);
        self.draft = ValidPlan::revalidate(next, false)?;
        self.last_change = Some(change.clone());
        Ok(change)
    }

    /// The operator's edit of one step. The id cannot change: that would be
    /// a new step, which is added with [`Workspace::insert_step`].
    pub fn replace_step(&mut self, step: Step, at: &str) -> Result<PlanDiff, WorkspaceError> {
        let mut next = self.draft.plan().clone();
        let slot = next
            .steps
            .iter_mut()
            .find(|s| s.id == step.id)
            .ok_or_else(|| WorkspaceError::UnknownStep(step.id.clone()))?;
        *slot = step;
        self.adopt(next, Some((operator(), ProvenanceAction::Edited, None)), at)
    }

    /// Add a step (for example a manual or validation step) after `after`,
    /// or at the start. It depends on `after`, so it runs after it.
    pub fn insert_step(
        &mut self,
        after: Option<&str>,
        mut step: Step,
        at: &str,
    ) -> Result<PlanDiff, WorkspaceError> {
        if self.draft.plan().step(&step.id).is_some() {
            return Err(WorkspaceError::DuplicateStep(step.id));
        }
        let mut next = self.draft.plan().clone();
        let position = match after {
            Some(a) => {
                let i = next
                    .steps
                    .iter()
                    .position(|s| s.id == a)
                    .ok_or_else(|| WorkspaceError::UnknownStep(a.into()))?;
                if !next.edges.is_empty() && !step.depends_on.iter().any(|d| d == a) {
                    step.depends_on.push(a.to_owned());
                }
                i + 1
            }
            None => 0,
        };
        next.steps.insert(position, step);
        self.adopt(
            next,
            Some((operator(), ProvenanceAction::Edited, Some("added by the operator".into()))),
            at,
        )
    }

    /// Remove a step. Refused, with the reason, while anything else refers
    /// to it: the operator changes those first.
    pub fn remove_step(&mut self, id: &str, at: &str) -> Result<PlanDiff, WorkspaceError> {
        let mut next = self.draft.plan().clone();
        if next.step(id).is_none() {
            return Err(WorkspaceError::UnknownStep(id.into()));
        }
        next.steps.retain(|s| s.id != id);
        next.edges.retain(|e| e.from != id && e.to != id);
        for p in &mut next.phases {
            p.steps.retain(|s| s != id);
        }
        self.adopt(
            next,
            Some((operator(), ProvenanceAction::Edited, Some(format!("removed step `{id}`")))),
            at,
        )
    }

    /// Move a step one place earlier or later in the list. Where the plan
    /// has no edges the list is the order; otherwise the graph decides and
    /// only display order changes. Refused where it would break the plan.
    pub fn move_step(&mut self, id: &str, earlier: bool, at: &str) -> Result<PlanDiff, WorkspaceError> {
        let mut next = self.draft.plan().clone();
        let i = next
            .steps
            .iter()
            .position(|s| s.id == id)
            .ok_or_else(|| WorkspaceError::UnknownStep(id.into()))?;
        let j = if earlier { i.checked_sub(1) } else { Some(i + 1).filter(|j| *j < next.steps.len()) };
        let Some(j) = j else { return Ok(PlanDiff::default()) };
        next.steps.swap(i, j);
        self.adopt(next, Some((operator(), ProvenanceAction::Edited, Some("moved".into()))), at)
    }

    /// Stage every artifact the draft needs and pin the hash of any
    /// the plan left unpinned. Pinning is an edit: the step goes back to
    /// validation, and the operator reviews the hash like any other change.
    pub fn stage(
        &mut self,
        store: &std::path::Path,
        at: &str,
    ) -> Result<Vec<crate::artifacts::StagedArtifact>, WorkspaceError> {
        let mut staged = Vec::new();
        for a in crate::artifacts::artifacts(self.draft.plan()) {
            staged.push(crate::artifacts::stage(store, a, at).map_err(WorkspaceError::Plan)?);
        }
        let pinned = crate::artifacts::pin(self.draft.plan(), &staged);
        if &pinned != self.draft.plan() {
            self.adopt(
                pinned,
                Some((
                    operator(),
                    ProvenanceAction::Edited,
                    Some("artifact hashes pinned by staging".into()),
                )),
                at,
            )?;
        }
        Ok(staged)
    }

    /// Validate the whole draft now and record the result.
    pub fn validate(&mut self, options: Options, at: &str) -> Result<(), WorkspaceError> {
        let report = validate(&self.draft, options);
        self.problems = report.problems.clone();
        self.draft = ValidPlan::revalidate(report.record_in(self.draft.plan(), at), false)?;
        self.last_change = None;
        Ok(())
    }

    /// Ask an agent for a plan for the task.
    pub fn propose<R: Runner>(
        agents: &Agents<'_, R>,
        agent: &AgentHandle,
        task: &str,
        context: &PreparedContext,
        at: &str,
    ) -> Result<Self, WorkspaceError> {
        let p = agents.propose(agent, task, context, at)?;
        let mut w = Self::new(task, p.plan);
        w.note("You", task, None, at);
        let who = agent_name(w.plan());
        if !p.summary.is_empty() {
            w.note(&who, &p.summary, None, at);
        }
        Ok(w)
    }

    /// Ask the agent to redo one step with the operator's guidance and what
    /// validation and review found about it. The replacement carries
    /// no approval and no validation.
    pub fn retry_step<R: Runner>(
        &mut self,
        agents: &Agents<'_, R>,
        agent: &AgentHandle,
        step: &str,
        guidance: &str,
        failure: Option<&keyjutsu_agent::RunFailure>,
        at: &str,
    ) -> Result<PlanDiff, WorkspaceError> {
        if self.draft.plan().step(step).is_none() {
            return Err(WorkspaceError::UnknownStep(step.into()));
        }
        let findings = self.findings_for(step);
        let request = StepRevision { step, guidance, findings: &findings, failure };
        let p = agents.revise_step(agent, &self.task, self.draft.plan(), &request, at)?;
        self.note("You", guidance, Some(step), at);
        let change = self.adopt(p.plan.into_plan(), None, at)?;
        let who = agent_name(self.plan());
        if !p.summary.is_empty() {
            self.note(&who, &p.summary, Some(step), at);
        }
        Ok(change)
    }

    /// Ask the agent to reconsider the whole plan.
    pub fn revise_plan<R: Runner>(
        &mut self,
        agents: &Agents<'_, R>,
        agent: &AgentHandle,
        guidance: &str,
        at: &str,
    ) -> Result<PlanDiff, WorkspaceError> {
        let p = agents.revise_plan(agent, &self.task, self.draft.plan(), guidance, at)?;
        self.note("You", guidance, None, at);
        let change = self.adopt(p.plan.into_plan(), None, at)?;
        let who = agent_name(self.plan());
        if !p.summary.is_empty() {
            self.note(&who, &p.summary, None, at);
        }
        Ok(change)
    }

    /// Answer the agent's question `id` with `answer`: the question and the
    /// answer go back to the agent as guidance for the whole plan, and what
    /// it returns is adopted like any revision, unvalidated and unapproved.
    /// The question is closed whatever the agent sends back, so it is never
    /// asked twice, and the answer is kept in the plan's provenance.
    pub fn answer<R: Runner>(
        &mut self,
        agents: &Agents<'_, R>,
        agent: &AgentHandle,
        id: &str,
        answer: &str,
        at: &str,
    ) -> Result<PlanDiff, WorkspaceError> {
        let answer = answer.trim();
        if answer.is_empty() {
            return Err(WorkspaceError::Plan("an answer needs some text".into()));
        }
        let question = self.open_question(id)?;
        let guidance = format!(
            "You asked: {}\nThe operator answered: {answer}\n\
             Revise the plan to follow this answer, and leave question \"{id}\" out of \"questions\".",
            question.text
        );
        let p = agents.revise_plan(agent, &self.task, self.draft.plan(), &guidance, at)?;
        let mut next = p.plan.into_plan();
        next.questions.retain(|q| q.id != id);
        if let Some(state) = next.keyjutsu.as_mut() {
            state.provenance.push(answered(&question, answer, at));
        }
        self.note("You", &format!("{}: {answer}", question.text), question.step.as_deref(), at);
        let change = self.adopt(next, None, at)?;
        let who = agent_name(self.plan());
        if !p.summary.is_empty() {
            self.note(&who, &p.summary, question.step.as_deref(), at);
        }
        Ok(change)
    }

    /// Close the agent's question `id` without asking the agent anything:
    /// the plan stays as it is, and the decision is kept in its provenance.
    /// No step changes, so nothing goes back to validation.
    pub fn carry_on(&mut self, id: &str, at: &str) -> Result<(), WorkspaceError> {
        let question = self.open_question(id)?;
        let mut plan = self.draft.plan().clone();
        plan.questions.retain(|q| q.id != id);
        plan.keyjutsu
            .get_or_insert_with(|| KeyJutsuState {
                revision: None,
                steps: BTreeMap::new(),
                snapshot_hash: None,
                provenance: Vec::new(),
            })
            .provenance
            .push(answered(&question, "carry on as planned", at));
        self.draft = ValidPlan::revalidate(plan, false)?;
        Ok(())
    }

    fn open_question(&self, id: &str) -> Result<keyjutsu_plan::model::Question, WorkspaceError> {
        self.draft
            .plan()
            .question(id)
            .cloned()
            .ok_or_else(|| WorkspaceError::Plan(format!("there is no open question `{id}`")))
    }

    /// Ask another agent to challenge the plan. It changes no step.
    pub fn review<R: Runner>(
        &mut self,
        agents: &Agents<'_, R>,
        reviewer: &AgentHandle,
        at: &str,
    ) -> Result<(), WorkspaceError> {
        let review = agents.review(reviewer, &self.task, self.draft.plan())?;
        let recorded = record_review(self.draft.plan(), reviewer, &review, at);
        self.draft = ValidPlan::revalidate(recorded, false)?;
        let who = format!(
            "{} review",
            serde_json::to_value(reviewer.kind.plan_name())
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default()
        );
        if !review.summary.is_empty() {
            self.note(&who, &review.summary, None, at);
        }
        for f in &review.findings {
            self.notes.push(Note {
                who: who.clone(),
                text: f.message.clone(),
                step: f.step.clone(),
                review: true,
                dismissed: false,
                at: at.into(),
            });
        }
        self.review = Some(review);
        Ok(())
    }

    /// What validation and review said about a step, for the agent.
    fn findings_for(&self, step: &str) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(s) = self.draft.plan().keyjutsu.as_ref().and_then(|k| k.steps.get(step)) {
            out.push(format!("readiness: {:?}", s.readiness));
            out.extend(s.risk_reasons.iter().map(|r| format!("risk: {r}")));
            out.extend(s.remaining_uncertainty.iter().map(|u| format!("uncertain: {u}")));
            out.extend(
                s.evidence
                    .iter()
                    .filter(|e| e.result == keyjutsu_plan::model::EvidenceResult::Failed)
                    .map(|e| format!("{}: {}", e.check, e.detail.as_deref().unwrap_or("failed"))),
            );
        }
        if let Some(r) = &self.review {
            out.extend(
                r.findings
                    .iter()
                    .filter(|f| f.step.as_deref() == Some(step))
                    .map(|f| format!("review: {}", f.message)),
            );
        }
        out
    }

    pub fn view(&self) -> WorkspaceView {
        let plan = self.draft.plan();
        let states = plan.keyjutsu.as_ref().map(|k| &k.steps);
        let mut steps = Vec::new();
        let mut overall = Overall {
            total: 0,
            ready: 0,
            needs_review: 0,
            blocked: 0,
            invalid: 0,
            unvalidated: 0,
            highest_risk: None,
            needs_elevation: false,
            recovery_prepared: true,
            blocking: Vec::new(),
        };
        for (i, id) in self.draft.graph().topological_order().enumerate() {
            let Some(step) = plan.step(id) else { continue };
            let state = states.and_then(|s| s.get(id));
            let readiness = state.map(|s| s.readiness);
            let risk = state.and_then(|s| s.assessed_risk).or(step.proposed_risk.as_ref().map(|r| r.level));
            let critical = is_critical(plan, step);
            overall.total += 1;
            match readiness {
                Some(Readiness::Ready) => overall.ready += 1,
                Some(Readiness::NeedsReview) => overall.needs_review += 1,
                Some(Readiness::Blocked) => overall.blocked += 1,
                Some(Readiness::Invalid) => overall.invalid += 1,
                None | Some(Readiness::RevalidationRequired) => overall.unvalidated += 1,
            }
            if risk > overall.highest_risk {
                overall.highest_risk = risk;
            }
            if step.privilege == Some(Privilege::Administrator) {
                overall.needs_elevation = true;
            }
            let reversible = step
                .reversibility
                .as_ref()
                .is_some_and(|r| r.level != keyjutsu_plan::model::ReversibilityLevel::None);
            if reversible && step.recovery.is_none() && step.kind == StepKind::Command {
                overall.recovery_prepared = false;
            }
            let concerns = self
                .review
                .as_ref()
                .map(|r| {
                    r.findings
                        .iter()
                        .filter(|f| f.step.as_deref() == Some(id) && f.severity != Severity::Info)
                        .count()
                })
                .unwrap_or(0);
            steps.push(StepSummary {
                id: id.to_owned(),
                number: u32::try_from(i + 1).unwrap_or(u32::MAX),
                title: step.title.clone(),
                detail: detail(step),
                readiness,
                risk,
                critical,
                confirmation_phrase: critical.then(|| confirmation_phrase(step)),
                concerns: u32::try_from(concerns).unwrap_or(u32::MAX),
            });
        }
        if overall.unvalidated > 0 {
            overall.blocking.push(format!(
                "{} validation",
                keyjutsu_plan::count(
                    usize::try_from(overall.unvalidated).unwrap_or(usize::MAX),
                    "step needs",
                    "steps need"
                )
            ));
        }
        let not_ready = overall.needs_review + overall.blocked + overall.invalid;
        if not_ready > 0 {
            overall.blocking.push(format!(
                "{} not ready",
                keyjutsu_plan::count(
                    usize::try_from(not_ready).unwrap_or(usize::MAX),
                    "step is",
                    "steps are"
                )
            ));
        }
        WorkspaceView {
            task: self.task.clone(),
            plan: plan.clone(),
            steps,
            overall,
            notes: self.notes.clone(),
            last_change: self.last_change.clone(),
            problems: self.problems.clone(),
            asks: crate::asks::asks(
                plan,
                &self.draft.graph().topological_order().collect::<Vec<_>>(),
                &self.notes,
            ),
        }
    }

    /// Approve every step and seal a snapshot. A critical step needs its
    /// phrase typed, as `confirmations[step]`; nothing is approved otherwise.
    pub fn approve(
        &self,
        confirmations: &BTreeMap<String, String>,
        fingerprint: Option<EnvironmentFingerprint>,
        at: &str,
    ) -> Result<ApprovedSnapshot, WorkspaceError> {
        let blocking = self.view().overall.blocking;
        if !blocking.is_empty() {
            return Err(WorkspaceError::Approval(format!("not yet: {}", blocking.join("; "))));
        }
        let mut book = ApprovalBook::new();
        for critical in book.approve_all_except_critical(&self.draft, at) {
            match book.approve(&self.draft, &critical, at, confirmations.get(&critical).map(String::as_str)) {
                Ok(()) => {}
                Err(
                    ApprovalError::ConfirmationRequired { step, phrase }
                    | ApprovalError::ConfirmationMismatch { step, phrase },
                ) => {
                    return Err(WorkspaceError::Approval(format!(
                        "critical step `{step}` needs its confirmation typed exactly: {phrase}"
                    )));
                }
                Err(e) => return Err(WorkspaceError::Approval(e.to_string())),
            }
        }
        seal(&self.draft, &book, fingerprint, at).map_err(|e| WorkspaceError::Approval(e.to_string()))
    }
}

fn detail(step: &Step) -> String {
    let mut parts = Vec::new();
    match step.kind {
        StepKind::Manual => parts.push("done by you".to_owned()),
        StepKind::UserInput => parts.push("your input".to_owned()),
        StepKind::Credential => parts.push("credential, typed by you".to_owned()),
        _ => {
            if let Some(shell) = &step.shell {
                parts.push(
                    match shell.kind {
                        keyjutsu_plan::model::ShellName::Pwsh => "PowerShell 7",
                        keyjutsu_plan::model::ShellName::WindowsPowershell => "Windows PowerShell",
                        keyjutsu_plan::model::ShellName::Cmd => "cmd",
                    }
                    .to_owned(),
                );
            }
        }
    }
    if step.privilege == Some(Privilege::Administrator) {
        parts.push("administrator".into());
    }
    if let Some(r) = &step.reversibility {
        parts.push(
            match r.level {
                keyjutsu_plan::model::ReversibilityLevel::Full => "reversible",
                keyjutsu_plan::model::ReversibilityLevel::Partial => "partly reversible",
                keyjutsu_plan::model::ReversibilityLevel::None => "not reversible",
            }
            .to_owned(),
        );
    }
    if step.kind == StepKind::Validation {
        parts.push("check only".into());
    }
    parts.join(" · ")
}

/// Where a snapshot and its runs are kept: `%LOCALAPPDATA%\KeyJutsu\runs`,
/// one folder per snapshot, named for the plan and the snapshot's hash.
pub fn run_folder(snapshot: &ApprovedSnapshot) -> std::path::PathBuf {
    let hash = snapshot.snapshot_hash();
    runs_root().join(format!("{}-{}", snapshot.plan().plan_id, &hash[..hash.len().min(12)]))
}

/// Where the desktop app keeps the plans it approved, one folder each.
pub fn runs_root() -> std::path::PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("KeyJutsu")
        .join("runs")
}
