//! The change record says what was approved under which profile, by whom,
//! what ran, and how it ended, from the session's record alone.

#![allow(clippy::unwrap_used)] // Helpers outside #[test] functions may unwrap too.

use keyjutsu_core::change_record::{build, save};
use keyjutsu_core::execute::{CheckResult, Checkpoint, Outcome, StepRun};
use keyjutsu_core::history::SessionRecord;
use keyjutsu_core::plan::{ApprovalBook, parse_plan, seal};
use serde_json::json;

const AT: &str = "2026-10-10T12:00:00Z";

fn record() -> SessionRecord {
    let v = json!({
        "schema_version": "1.0", "plan_id": "spooler", "task_id": "t", "title": "Restart the spooler",
        "target": {"id": "local", "kind": "local_windows"},
        "agent": {"name": "claude_code", "version": "2.1"},
        "steps": [
            {"id": "look", "title": "Read its state", "objective": "Seen.", "kind": "validation",
             "shell": {"kind": "pwsh"}, "commands": [{"text": "Get-Service -Name Spooler"}]},
            {"id": "stop", "title": "Stop the spooler", "objective": "Stopped.", "kind": "command",
             "shell": {"kind": "pwsh"}, "commands": [{"text": "Stop-Service -Name Spooler"}],
             "recovery": {"strategy": "commands", "commands": [{"text": "Start-Service -Name Spooler"}]}}
        ],
        "keyjutsu": {
            "revision": 1,
            "steps": {
                "look": {"readiness": "READY", "proof_level": "HIGH", "assessed_risk": "low"},
                "stop": {"readiness": "READY", "proof_level": "MEDIUM", "assessed_risk": "high"}
            },
            "work_profile": {
                "name": "it-operations", "title": "IT Operations",
                "chain": [
                    {"name": "it-operations", "origin": "built into KeyJutsu", "hash": "a".repeat(64)},
                    {"name": "helpdesk", "origin": "built into KeyJutsu"}
                ],
                "why": "This machine's default.",
                "confirm_each_from": "high",
                "started_in": "Q:\\work", "real_path": "\\\\server\\share\\work",
                "leftovers": ["desktop.ini"]
            }
        }
    });
    let plan = parse_plan(&v.to_string()).unwrap();
    let mut book = ApprovalBook::new();
    assert_eq!(book.approve_all_except_critical(&plan, AT), ["stop"]);
    book.approve(&plan, "stop", AT, Some("STOP THE SPOOLER")).unwrap();
    let snapshot = seal(&plan, &book, None, AT).unwrap();
    let hash = snapshot.step_hashes()["look"].clone();
    let mut checkpoint = Checkpoint::new(snapshot.snapshot_hash());
    checkpoint.runs.push(StepRun {
        step: "look".into(),
        step_hash: hash,
        succeeded: true,
        exit_code: Some(0),
        checks: vec![CheckResult { check: "exit code".into(), passed: Some(true), detail: "0".into() }],
        started_at: AT.into(),
        finished_at: AT.into(),
    });
    SessionRecord {
        id: "20261010-120000-abcd".into(),
        started_at: AT.into(),
        finished_at: "2026-10-10T12:01:00Z".into(),
        task: "Restart the spooler".into(),
        agent: snapshot.plan().agent.clone(),
        snapshot: snapshot.to_json(),
        checkpoint: Some(checkpoint),
        outcome: Outcome::Failed {
            step: "stop".into(),
            expected: "exit code 0".into(),
            actual: "exit code 1".into(),
            output: "Stop-Service: Cannot stop service 'Print Spooler' token=ghp_abcdefghijklmnopqrstuvwxyz0123456789\n".into(),
        },
        git: Vec::new(),
        identity: Some("CONTOSO\\operator".into()),
    }
}

#[test]
fn a_change_record_says_what_was_approved_under_what_and_what_happened() {
    let change = build(&record(), None).unwrap();
    assert_eq!(change.identity.as_deref(), Some("CONTOSO\\operator"));
    let results: Vec<(&str, &str)> =
        change.steps.iter().map(|s| (s.id.as_str(), s.result.as_str())).collect();
    assert_eq!(results, [("look", "succeeded"), ("stop", "failed")]);
    // The failed step's output is kept with the outcome, redacted.
    let printed = change.steps[1].printed.as_deref().unwrap();
    assert!(printed.starts_with("Stop-Service: Cannot stop service 'Print Spooler'"), "{printed}");
    assert!(!printed.contains("ghp_abcdefghij"), "{printed}");
    assert_eq!(
        change.steps[1].confirmation.as_deref(),
        Some("IT Operations asks for its own confirmation from high risk")
    );
    assert_eq!(change.steps[0].confirmation, None);

    let md = change.markdown();
    for expected in [
        "# Change record: Restart the spooler",
        "- **Run as:** CONTOSO\\operator",
        "- **Planned by:** claude_code 2.1",
        "- **Outcome:** Failed at `stop`: expected exit code 0, got exit code 1.",
        "**IT Operations** (`it-operations`): This machine's default.",
        "Each step at high risk or above was confirmed on its own.",
        &format!("- `it-operations`: built into KeyJutsu, sha256 {}", "a".repeat(64)),
        "Started in `Q:\\work`, which is really `\\\\server\\share\\work`.",
        "Ignored in `.keyjutsu`: desktop.ini.",
        "### 2. Stop the spooler (`stop`): failed",
        "Printed:\n\n```text\nStop-Service: Cannot stop service",
        "- **Approved:** 2026-10-10T12:00:00Z, with its own typed confirmation (IT Operations asks for its own confirmation from high risk)",
        "- **Recovery:** run `Start-Service -Name Spooler`",
        "- **Check** exit code: passed, 0",
        "What the steps printed is not included, apart from a failed step's: the run was not recorded.",
    ] {
        assert!(md.contains(expected), "missing {expected:?} in\n{md}");
    }
}

#[test]
fn a_saved_change_record_is_named_after_its_session_and_replaced_by_the_next() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("change-record-save");
    let _ = std::fs::remove_dir_all(&dir);
    let md = build(&record(), None).unwrap().markdown();
    let file = save("20261010-120000-abcd", "old", &dir).unwrap();
    assert_eq!(save("20261010-120000-abcd", &md, &dir).unwrap(), file);
    assert_eq!(file.file_name().unwrap(), "change-record-20261010-120000-abcd.md");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), md);
    // Only a session id names a file: nothing can be written elsewhere.
    for bad in ["", "..", "a/b", "..\\up"] {
        assert!(save(bad, &md, &dir).is_err(), "{bad:?}");
    }
}

#[test]
fn a_record_without_an_identity_or_profile_says_so() {
    let mut r = record();
    r.identity = None;
    let md = build(&r, None).unwrap().markdown();
    assert!(md.contains("- **Run as:** not recorded"), "{md}");
}
