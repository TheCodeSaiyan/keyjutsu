//! The change record for one run: what was planned, under which work profile
//! and why, who approved what, what ran, what it printed, how it ended, and
//! how each step would be undone. Built on request from the session's
//! encrypted record, never kept in plain text unless the operator writes it
//! out, so it can be attached to a ticket.

use keyjutsu_plan::approval::confirmation_reason;
use keyjutsu_plan::model::{RecoveryStrategy, RiskLevel, WorkProfileRecord};
use serde::Serialize;

use crate::execute::{CheckResult, Outcome};
use crate::git::RepoReport;
use crate::history::SessionRecord;
use crate::recording::Recording;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChangeRecord {
    pub session: String,
    pub task: String,
    pub agent: String,
    pub started_at: String,
    pub finished_at: String,
    /// The Windows account the run used, when the record says.
    pub identity: Option<String>,
    pub outcome: String,
    pub work_profile: Option<WorkProfileRecord>,
    pub snapshot_hash: String,
    pub sealed_at: String,
    pub steps: Vec<StepRecord>,
    pub git: Vec<RepoReport>,
    /// Whether what each step printed is included: only a recorded run has it.
    pub output_recorded: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StepRecord {
    pub id: String,
    pub title: String,
    pub commands: Vec<String>,
    pub assessed_risk: Option<RiskLevel>,
    pub approved_at: Option<String>,
    /// Why it needed its own typed confirmation, when it did.
    pub confirmation: Option<String>,
    pub recovery: String,
    /// "succeeded", "failed", "in doubt" or "not run".
    pub result: String,
    pub exit_code: Option<i64>,
    pub checks: Vec<CheckResult>,
    pub printed: Option<String>,
}

/// The change record for `record`, with what each step printed when the run
/// was recorded. Secrets are redacted from output as in every export; names
/// are kept, since saying who did what is the point.
pub fn build(record: &SessionRecord, recording: Option<&Recording>) -> Result<ChangeRecord, String> {
    let snapshot = record.snapshot()?;
    let plan = snapshot.plan();
    let printed: Vec<(String, String)> = match recording {
        Some(r) => crate::recording::prepare_export(record, r, None, None, &[])?
            .steps
            .into_iter()
            .map(|s| (s.step, s.printed))
            .collect(),
        None => Vec::new(),
    };
    let runs = record.checkpoint.as_ref().map(|c| c.runs.as_slice()).unwrap_or_default();
    let in_progress =
        record.checkpoint.as_ref().and_then(|c| c.in_progress.as_ref()).map(|p| p.step.as_str());
    let steps = snapshot
        .graph()
        .topological_order()
        .filter_map(|id| plan.step(id))
        .map(|step| {
            let run = runs.iter().rev().find(|r| r.step == step.id);
            let failed_here = matches!(&record.outcome, Outcome::Failed { step: s, .. } if *s == step.id);
            let result = match run {
                Some(r) if r.succeeded => "succeeded",
                Some(_) => "failed",
                None if failed_here => "failed",
                None if in_progress == Some(step.id.as_str()) => "in doubt",
                None => "not run",
            };
            // A recorded run shows what every step printed; without one, the
            // failed step's output is still kept with the outcome.
            let printed = printed.iter().find(|(id, _)| *id == step.id).map(|(_, p)| p.clone()).or_else(
                || match &record.outcome {
                    Outcome::Failed { step: s, output, .. } if *s == step.id && !output.trim().is_empty() => {
                        Some(crate::agent::context::redact(output).0)
                    }
                    _ => None,
                },
            );
            let approval = snapshot.approvals().iter().find(|a| a.step == step.id);
            StepRecord {
                id: step.id.clone(),
                title: step.title.clone(),
                commands: step.commands.iter().map(|c| c.text.clone()).collect(),
                assessed_risk: plan
                    .keyjutsu
                    .as_ref()
                    .and_then(|k| k.steps.get(&step.id))
                    .and_then(|s| s.assessed_risk),
                approved_at: approval.map(|a| a.at.clone()),
                confirmation: approval
                    .and_then(|a| a.confirmation.as_ref())
                    .and(confirmation_reason(plan, step)),
                recovery: recovery(step),
                result: result.into(),
                exit_code: run.and_then(|r| r.exit_code),
                checks: run.map(|r| r.checks.clone()).unwrap_or_default(),
                printed,
            }
        })
        .collect();
    Ok(ChangeRecord {
        session: record.id.clone(),
        task: record.task.clone(),
        agent: format!(
            "{} {}",
            serde_json::to_value(record.agent.name)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default(),
            record.agent.version
        ),
        started_at: record.started_at.clone(),
        finished_at: record.finished_at.clone(),
        identity: record.identity.clone(),
        outcome: outcome(&record.outcome),
        work_profile: plan.keyjutsu.as_ref().and_then(|k| k.work_profile.clone()),
        snapshot_hash: snapshot.snapshot_hash().to_owned(),
        sealed_at: snapshot.sealed_at().to_owned(),
        steps,
        git: record.git.clone(),
        output_recorded: recording.is_some(),
    })
}

fn recovery(step: &keyjutsu_plan::model::Step) -> String {
    match &step.recovery {
        Some(r) if r.strategy == RecoveryStrategy::RestoreCapturedState => {
            let what: Vec<&str> = r.capture.iter().map(|c| c.target.as_str()).collect();
            format!("KeyJutsu restores what it captured first: {}", what.join(", "))
        }
        Some(r) if r.strategy == RecoveryStrategy::Commands => {
            let commands: Vec<&str> = r.commands.iter().map(|c| c.text.as_str()).collect();
            format!("run {}", commands.iter().map(|c| format!("`{c}`")).collect::<Vec<_>>().join(", then "))
        }
        _ => match step.reversibility.as_ref().and_then(|r| r.notes.as_deref()) {
            Some(notes) => format!("none declared ({notes})"),
            None => "none declared".into(),
        },
    }
}

fn outcome(o: &Outcome) -> String {
    match o {
        Outcome::Complete => "Complete: every step ran and passed its checks.".into(),
        Outcome::Failed { step, expected, actual, .. } => {
            format!("Failed at `{step}`: expected {expected}, got {actual}.")
        }
        Outcome::Aborted { step: Some(step), in_doubt: true } => {
            format!("Stopped by the operator during `{step}`; its effect is not known.")
        }
        Outcome::Aborted { step: Some(step), in_doubt: false } => {
            format!("Stopped by the operator at `{step}`.")
        }
        Outcome::Aborted { step: None, .. } => "Stopped by the operator.".into(),
        Outcome::Blocked { reason } => format!("Blocked: {reason}."),
        other => format!("{other:?}"),
    }
}

/// `text` with every backtick doubled into a fence that cannot close early.
fn fenced(text: &str) -> String {
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    format!("{fence}text\n{}\n{fence}", text.trim_end())
}

fn level(r: RiskLevel) -> &'static str {
    match r {
        RiskLevel::Low => "low",
        RiskLevel::Normal => "normal",
        RiskLevel::High => "high",
        RiskLevel::Critical => "critical",
    }
}

impl ChangeRecord {
    /// The record as Markdown, to paste into a ticket.
    pub fn markdown(&self) -> String {
        let mut s = format!("# Change record: {}\n\n", self.task);
        let mut row = |k: &str, v: &str| s.push_str(&format!("- **{k}:** {v}\n"));
        row("Session", &self.session);
        row("Started", &self.started_at);
        row("Finished", &self.finished_at);
        row("Run as", self.identity.as_deref().unwrap_or("not recorded"));
        row("Planned by", &self.agent);
        row("Outcome", &self.outcome);
        row("Approved", &format!("{} (snapshot {})", self.sealed_at, self.snapshot_hash));
        s.push('\n');

        s.push_str("## Work profile\n\n");
        match &self.work_profile {
            None => s.push_str("None was applied.\n\n"),
            Some(p) => {
                s.push_str(&format!("**{}** (`{}`)", p.title, p.name));
                if let Some(why) = &p.why {
                    s.push_str(&format!(": {why}"));
                }
                s.push_str("\n\n");
                if let Some(from) = p.confirm_each_from {
                    s.push_str(&format!(
                        "Each step at {} risk or above was confirmed on its own.\n\n",
                        level(from)
                    ));
                }
                for l in &p.chain {
                    let hash = l.hash.as_deref().map(|h| format!(", sha256 {h}")).unwrap_or_default();
                    s.push_str(&format!("- `{}`: {}{hash}\n", l.name, l.origin));
                }
                if let Some(started) = &p.started_in {
                    s.push_str(&format!("\nStarted in `{started}`"));
                    match &p.real_path {
                        Some(real) if real != started => s.push_str(&format!(", which is really `{real}`")),
                        _ => {}
                    }
                    s.push_str(".\n");
                }
                if let Some(hash) = &p.folder_hash {
                    s.push_str(&format!("The folder's `.keyjutsu` files hashed to {hash}.\n"));
                }
                if !p.leftovers.is_empty() {
                    s.push_str(&format!("Ignored in `.keyjutsu`: {}.\n", p.leftovers.join(", ")));
                }
                s.push('\n');
            }
        }

        s.push_str("## Steps\n\n");
        for (i, step) in self.steps.iter().enumerate() {
            s.push_str(&format!("### {}. {} (`{}`): {}\n\n", i + 1, step.title, step.id, step.result));
            if let Some(r) = step.assessed_risk {
                s.push_str(&format!("- **Risk:** {}\n", level(r)));
            }
            match (&step.approved_at, &step.confirmation) {
                (Some(at), Some(why)) => {
                    s.push_str(&format!("- **Approved:** {at}, with its own typed confirmation ({why})\n"))
                }
                (Some(at), None) => s.push_str(&format!("- **Approved:** {at}, with the plan\n")),
                (None, _) => s.push_str("- **Approved:** not recorded\n"),
            }
            s.push_str(&format!("- **Recovery:** {}\n", step.recovery));
            if let Some(code) = step.exit_code {
                s.push_str(&format!("- **Exit code:** {code}\n"));
            }
            for c in &step.checks {
                let mark = match c.passed {
                    Some(true) => "passed",
                    Some(false) => "FAILED",
                    None => "undecided",
                };
                s.push_str(&format!("- **Check** {}: {mark}, {}\n", c.check, c.detail));
            }
            s.push_str("\nRan:\n\n");
            s.push_str(&fenced(&step.commands.join("\n")));
            s.push_str("\n\n");
            if let Some(printed) = step.printed.as_deref().filter(|p| !p.trim().is_empty()) {
                s.push_str("Printed:\n\n");
                s.push_str(&fenced(printed));
                s.push_str("\n\n");
            }
        }
        if !self.output_recorded {
            s.push_str(
                "What the steps printed is not included, apart from a failed step's: the run was not recorded.\n\n",
            );
        }

        if !self.git.is_empty() {
            s.push_str("## Git\n\n");
            for r in &self.git {
                let changed: Vec<&str> = r.keyjutsu.iter().map(|c| c.path.as_str()).collect();
                let what = if changed.is_empty() {
                    "nothing changed".to_owned()
                } else {
                    format!("changed {}", changed.join(", "))
                };
                s.push_str(&format!("- `{}`: {what}\n", r.root));
                if let Some((before, after)) = &r.head_moved {
                    s.push_str(&format!("  - HEAD moved from {before} to {after}\n"));
                }
            }
            s.push('\n');
        }
        s.trim_end().to_owned() + "\n"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fence_cannot_be_closed_by_what_it_holds() {
        let f = fenced("one ``` two");
        assert!(f.starts_with("````text\n"), "{f}");
        assert!(f.ends_with("\n````"), "{f}");
    }
}
