//! Session history: what was asked, what was approved, what happened.
//!
//! A session is recorded when a plan run ends, unless the operator chose an
//! ephemeral session. The record keeps the sealed snapshot itself, so what
//! was approved can be checked again later exactly as it was, and reopening
//! compares today's machine with the one it was approved on.

use keyjutsu_plan::approval::ApprovedSnapshot;
use keyjutsu_plan::hash::{Drift, EnvironmentFingerprint, affected_by_drift};
use keyjutsu_plan::model::Agent;
use serde::{Deserialize, Serialize};

use crate::execute::{Checkpoint, Outcome};
use crate::git::RepoReport;
use crate::store::Store;

pub const KIND: &str = "session";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "history/")]
pub struct SessionRecord {
    pub id: String,
    pub started_at: String,
    pub finished_at: String,
    pub task: String,
    pub agent: Agent,
    /// The sealed snapshot, exactly as approved.
    pub snapshot: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub checkpoint: Option<Checkpoint>,
    pub outcome: Outcome,
    #[serde(default)]
    pub git: Vec<RepoReport>,
}

/// One line of the history list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "history/")]
pub struct SessionSummary {
    pub id: String,
    pub finished_at: String,
    pub task: String,
    pub outcome: String,
}

impl SessionRecord {
    pub fn snapshot(&self) -> Result<ApprovedSnapshot, String> {
        ApprovedSnapshot::from_json(&self.snapshot)
            .map_err(|e| format!("the recorded snapshot does not verify: {e}"))
    }

    pub fn succeeded(&self) -> bool {
        self.outcome == Outcome::Complete
    }
}

/// Keep `record`, with anything that looks like a secret taken out of what
/// the run printed and of KeyJutsu's diffs first. The store is encrypted, but
/// that is no reason to keep a token a failing command happened to print:
/// what history is for does not need it.
pub fn save(store: &Store, record: &SessionRecord) -> Result<(), String> {
    let clean = |text: &str| keyjutsu_agent::context::redact(text).0;
    let mut record = record.clone();
    if let Outcome::Failed { expected, actual, output, .. } = &mut record.outcome {
        *expected = clean(expected);
        *actual = clean(actual);
        *output = clean(output);
    }
    for change in record.git.iter_mut().flat_map(|r| r.keyjutsu.iter_mut()) {
        change.diff = clean(&change.diff);
    }
    store.put(KIND, &record.id, &record)
}

pub fn load(store: &Store, id: &str) -> Result<SessionRecord, String> {
    store.get(KIND, id)?.ok_or_else(|| format!("there is no session `{id}`"))
}

pub fn list(store: &Store) -> Result<Vec<SessionSummary>, String> {
    let mut out = Vec::new();
    for id in store.list(KIND)? {
        let r: SessionRecord = load(store, &id)?;
        let outcome = match &r.outcome {
            Outcome::Complete => "complete".to_owned(),
            Outcome::Failed { step, .. } => format!("failed at {step}"),
            Outcome::Aborted { .. } => "disarmed".to_owned(),
            Outcome::Blocked { .. } => "blocked".to_owned(),
            Outcome::Boundary { boundary, .. } => {
                format!("waiting: {}", crate::boundary::describe(*boundary))
            }
        };
        out.push(SessionSummary { id: r.id, finished_at: r.finished_at, task: r.task, outcome });
    }
    Ok(out)
}

/// What has changed on this machine since a recorded session was approved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "history/")]
pub struct Recheck {
    pub drifts: Vec<Drift>,
    /// Steps whose approval relied on something that has changed.
    pub affected: Vec<String>,
    /// The session was approved without a fingerprint to compare with.
    pub no_fingerprint: bool,
}

/// Compare `now` with the environment a recorded session was approved on.
pub fn recheck(record: &SessionRecord, now: &EnvironmentFingerprint) -> Result<Recheck, String> {
    let snap = record.snapshot()?;
    let Some(then) = snap.fingerprint() else {
        return Ok(Recheck { drifts: Vec::new(), affected: Vec::new(), no_fingerprint: true });
    };
    let drifts = then.drift(now);
    let affected = affected_by_drift(snap.plan(), snap.graph(), &drifts);
    Ok(Recheck { drifts, affected, no_fingerprint: false })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::KeyJutsuChange;

    #[test]
    fn a_secret_a_failing_command_printed_is_not_kept_in_the_history() {
        let token = "ghp_0123456789abcdefghijklmnopqrstuvwxyzAB";
        let dir = std::env::temp_dir().join(format!("kj-history-{}", std::process::id()));
        let store = Store::open(&dir).unwrap();
        let record = SessionRecord {
            id: "s1".into(),
            started_at: "2026-09-27T00:00:00Z".into(),
            finished_at: "2026-09-27T00:01:00Z".into(),
            task: "t".into(),
            agent: serde_json::from_value(serde_json::json!({"name": "codex", "version": "1"})).unwrap(),
            snapshot: "{}".into(),
            checkpoint: None,
            outcome: Outcome::Failed {
                step: "push".into(),
                expected: "exit code 0".into(),
                actual: format!("exit code 1, token={token}"),
                output: format!("remote: Invalid credentials for {token}\npassword=hunter2\n"),
            },
            git: vec![RepoReport {
                root: "C:/repo".into(),
                branch: None,
                keyjutsu: vec![KeyJutsuChange {
                    path: ".env".into(),
                    was_already_changed: false,
                    status: "??".into(),
                    diff: format!("+GITHUB_TOKEN={token}\n+API_KEY=sk-proj-abcdefghijklmnopqrstuvwxyz\n"),
                }],
                untouched: vec![],
                head_moved: None,
            }],
        };
        save(&store, &record).unwrap();
        let kept = serde_json::to_string(&load(&store, "s1").unwrap()).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        for secret in [token, "hunter2", "sk-proj-abcdefghijklmnopqrstuvwxyz"] {
            assert!(!kept.contains(secret), "{secret} was kept: {kept}");
        }
        assert!(
            kept.contains("Invalid credentials") && kept.contains("[REDACTED"),
            "the rest is kept: {kept}"
        );
    }
}
