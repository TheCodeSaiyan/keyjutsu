//! The plan workspace. Validation is real (pwsh); agents are
//! replayed from recorded answers, as in the agent crate's tests.

#![cfg(windows)]
#![allow(clippy::unwrap_used)] // Helpers outside #[test] functions may unwrap too.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;

use keyjutsu_core::agent::agents::Invocation;
use keyjutsu_core::agent::context::prepare;
use keyjutsu_core::agent::session::RunOutput;
use keyjutsu_core::agent::{AgentError, AgentHandle, AgentKind, Agents, Runner};
use keyjutsu_core::plan::model::{Command, ProvenanceAction, Readiness, Step};
use keyjutsu_core::plan::parse_plan;
use keyjutsu_core::validation::Options;
use keyjutsu_core::workspace::{Workspace, WorkspaceError};
use serde_json::{Value, json};

const AT: &str = "2026-09-25T06:00:00Z";

fn step(id: &str, command: &str, after: &[&str]) -> Value {
    json!({"id": id, "title": format!("Step {id}"), "objective": "Test.", "kind": "command",
           "shell": {"kind": "pwsh"}, "commands": [{"text": command}], "depends_on": after})
}

/// a -> b -> c, and d on its own after a.
fn chain() -> Value {
    json!({
        "schema_version": "1.0", "plan_id": "p", "task_id": "t", "title": "Look around",
        "target": {"id": "local", "kind": "local_windows"},
        "agent": {"name": "codex", "version": "1"},
        "steps": [step("a", "Get-Date", &[]), step("b", "Get-Location", &["a"]),
                  step("c", "Get-Host", &["b"]), step("d", "Get-Culture", &["a"])],
        "edges": [{"from": "a", "to": "b"}, {"from": "b", "to": "c"}, {"from": "a", "to": "d"}]
    })
}

fn open(v: &Value) -> Workspace {
    Workspace::open(&v.to_string()).unwrap()
}

fn validated(v: &Value) -> Workspace {
    let mut w = open(v);
    w.validate(Options { dry_run: false, ..Options::default() }, AT).unwrap();
    w
}

fn readiness(w: &Workspace) -> BTreeMap<String, Option<Readiness>> {
    w.view().steps.into_iter().map(|s| (s.id, s.readiness)).collect()
}

#[test]
fn a_new_plan_needs_validation_before_it_can_be_approved() {
    let w = open(&chain());
    let v = w.view();
    assert_eq!(v.overall.unvalidated, 4);
    assert!(v.overall.blocking.iter().any(|b| b.contains("need validation")), "{:?}", v.overall.blocking);
    assert!(matches!(w.approve(&BTreeMap::new(), None, AT), Err(WorkspaceError::Approval(_))));
}

#[test]
fn editing_a_step_sends_it_and_what_follows_back_to_validation() {
    let mut w = validated(&chain());
    assert!(readiness(&w).values().all(|r| *r == Some(Readiness::Ready)), "{:?}", readiness(&w));

    let mut b: Step = w.plan().step("b").unwrap().clone();
    b.commands = vec![Command { text: "Get-Location | Select-Object Path".into(), purpose: None }];
    let change = w.replace_step(b, AT).unwrap();

    assert_eq!(change.affected, ["b", "c"]);
    let r = readiness(&w);
    assert_eq!(
        (r["a"], r["d"]),
        (Some(Readiness::Ready), Some(Readiness::Ready)),
        "unaffected steps keep theirs"
    );
    assert_eq!((r["b"], r["c"]), (None, None));
    let v = w.view();
    assert_eq!(v.overall.unvalidated, 2);
    assert!(!v.overall.blocking.is_empty());
    let last = w.plan().keyjutsu.as_ref().unwrap().provenance.last().unwrap();
    assert_eq!((last.step.as_deref(), last.action), (Some("b"), ProvenanceAction::Edited));
}

#[test]
fn renaming_a_step_keeps_its_validation() {
    let mut w = validated(&chain());
    let mut b = w.plan().step("b").unwrap().clone();
    b.title = "Where am I".into();
    let change = w.replace_step(b, AT).unwrap();
    assert!(change.affected.is_empty());
    assert!(readiness(&w).values().all(|r| *r == Some(Readiness::Ready)));
}

#[test]
fn an_edit_the_plan_would_refuse_leaves_the_draft_as_it_was() {
    let mut w = validated(&chain());
    let before = w.plan().clone();
    let mut b = w.plan().step("b").unwrap().clone();
    b.depends_on = vec!["nowhere".into()];
    let err = w.replace_step(b, AT).unwrap_err();
    assert!(err.to_string().contains("nowhere"), "{err}");
    let mut c = w.plan().step("c").unwrap().clone();
    c.commands = vec![Command { text: "Get-Date\rRemove-Item x".into(), purpose: None }];
    assert!(w.replace_step(c, AT).is_err(), "a control character in a command");
    assert_eq!(w.plan(), &before);
}

#[test]
fn steps_can_be_added_and_removed_but_not_out_from_under_others() {
    let mut w = validated(&chain());
    let manual: Step = serde_json::from_value(json!({"id": "check-ui", "title": "Look at the tray icon",
        "objective": "Docker's whale is steady.", "kind": "manual"}))
    .unwrap();
    let change = w.insert_step(Some("b"), manual, AT).unwrap();
    assert_eq!(change.added, ["check-ui"]);
    assert_eq!(w.plan().step("check-ui").unwrap().depends_on, ["b"]);
    let order: Vec<String> = w.view().steps.into_iter().map(|s| s.id).collect();
    assert!(order.iter().position(|s| s == "check-ui") > order.iter().position(|s| s == "b"));

    let err = w.remove_step("b", AT).unwrap_err();
    assert!(err.to_string().contains('b'), "{err}");
    assert!(w.plan().step("b").is_some());
    w.remove_step("check-ui", AT).unwrap();
    w.remove_step("d", AT).unwrap();
    assert!(w.plan().step("d").is_none());
    assert!(matches!(w.remove_step("zzz", AT), Err(WorkspaceError::UnknownStep(_))));
}

fn critical_plan() -> Value {
    json!({
        "schema_version": "1.0", "plan_id": "p", "task_id": "t",
        "target": {"id": "local", "kind": "local_windows"},
        "agent": {"name": "codex", "version": "1"},
        "steps": [
            {"id": "wipe", "title": "Remove scratch data", "objective": "Start clean.", "kind": "command",
             "shell": {"kind": "pwsh"},
             "commands": [{"text": "Remove-Item -Recurse -Force -LiteralPath C:/KeyJutsu-does-not-exist/scratch"}]}
        ]
    })
}

#[test]
fn a_critical_step_is_approved_only_with_its_phrase() {
    let mut w = open(&critical_plan());
    w.validate(Options { dry_run: false, ..Options::default() }, AT).unwrap();
    let view = w.view();
    let wipe = &view.steps[0];
    assert!(wipe.critical, "{wipe:?}");
    let phrase = wipe.confirmation_phrase.clone().unwrap();
    assert_eq!(phrase, "REMOVE SCRATCH DATA");
    assert!(view.overall.blocking.is_empty(), "{:?}", view.overall.blocking);
    let err = w.approve(&BTreeMap::new(), None, AT).unwrap_err();
    assert!(err.to_string().contains(&phrase), "{err}");
    let wrong = BTreeMap::from([("wipe".to_owned(), "yes".to_owned())]);
    assert!(w.approve(&wrong, None, AT).is_err());
    let right = BTreeMap::from([("wipe".to_owned(), phrase)]);
    let snap = w.approve(&right, None, AT).unwrap();
    assert_eq!(snap.approvals()[0].confirmation.as_deref(), Some("REMOVE SCRATCH DATA"));
}

#[test]
fn an_approved_snapshot_is_the_draft_as_validated() {
    let w = validated(&chain());
    let snap = w.approve(&BTreeMap::new(), None, AT).unwrap();
    assert_eq!(snap.plan().steps, w.plan().steps);
    assert!(parse_plan(&serde_json::to_string(snap.plan()).unwrap()).is_ok());
}

/// Replays answers in order.
struct Replay(RefCell<Vec<RunOutput>>, RefCell<Vec<Invocation>>);

impl Runner for Replay {
    fn run(&self, inv: &Invocation) -> Result<RunOutput, AgentError> {
        self.1.borrow_mut().push(inv.clone());
        self.0.borrow_mut().pop().ok_or_else(|| AgentError::Run("no more answers".into()))
    }
}

fn claude_says(doc: &Value, summary: &str) -> RunOutput {
    let message = format!("```json\n{doc}\n```\n{summary}");
    RunOutput {
        success: true,
        stdout: json!({"type": "result", "is_error": false, "result": message}).to_string(),
        ..RunOutput::default()
    }
}

fn claude() -> AgentHandle {
    AgentHandle { kind: AgentKind::ClaudeCode, program: PathBuf::from("claude.exe"), version: "2.1".into() }
}

#[test]
fn an_agent_proposal_becomes_a_workspace_with_the_conversation_beside_it() {
    let runner =
        Replay(RefCell::new(vec![claude_says(&chain(), "Four read-only looks.")]), RefCell::default());
    let agents = Agents { runner: &runner, scratch: std::env::temp_dir(), max_repairs: 1 };
    let w = Workspace::propose(&agents, &claude(), "Look around", &prepare(&[]).unwrap(), AT).unwrap();
    let v = w.view();
    assert_eq!(v.steps.len(), 4);
    assert_eq!(v.notes.iter().map(|n| n.who.as_str()).collect::<Vec<_>>(), ["You", "claude_code"]);
    assert_eq!(v.notes[1].text, "Four read-only looks.");
}

#[test]
fn retrying_a_step_gives_the_agent_the_findings_and_discards_validation() {
    let mut w = validated(&chain());
    let mut revised = chain();
    revised["steps"][1]["commands"][0]["text"] = json!("Get-Location | Format-List");
    let runner =
        Replay(RefCell::new(vec![claude_says(&revised["steps"][1], "Formatted.")]), RefCell::default());
    let agents = Agents { runner: &runner, scratch: std::env::temp_dir(), max_repairs: 1 };
    let change = w.retry_step(&agents, &claude(), "b", "Show it as a list.", None, AT).unwrap();

    assert_eq!(w.plan().step("b").unwrap().commands[0].text, "Get-Location | Format-List");
    assert!(change.affected.contains(&"b".to_owned()));
    let prompt = &runner.1.borrow()[0].stdin;
    assert!(prompt.contains("Show it as a list.") && prompt.contains("readiness"), "the findings go with it");
    let r = readiness(&w);
    assert_eq!(r["b"], None, "the replacement is not validated");
    let notes: Vec<String> = w.view().notes.into_iter().map(|n| n.text).collect();
    assert_eq!(notes, ["Show it as a list.", "Formatted."]);
}

/// The V1 recovery loop: a step failed when it ran, and the agent asked to
/// fix it is shown what it printed, redacted, as well as the guidance.
#[test]
fn fixing_a_failed_step_shows_the_agent_what_it_printed() {
    let mut w = validated(&chain());
    let mut revised = chain();
    revised["steps"][1]["commands"][0]["text"] = json!("Get-Location | Format-List");
    let runner = Replay(
        RefCell::new(vec![claude_says(&revised["steps"][1], "It needed a list.")]),
        RefCell::default(),
    );
    let agents = Agents { runner: &runner, scratch: std::env::temp_dir(), max_repairs: 1 };
    let failure = keyjutsu_core::agent::RunFailure {
        expected: "exit code 0".into(),
        actual: "a command exited with 1".into(),
        output: "Get-Location : the path is locked\napi_key=abcdefghijklmnopqrstuvwxyz0123".into(),
    };
    w.retry_step(&agents, &claude(), "b", "Try another way.", Some(&failure), AT).unwrap();

    let prompt = &runner.1.borrow()[0].stdin;
    assert!(prompt.contains("the path is locked"), "{prompt}");
    assert!(prompt.contains("a command exited with 1"));
    assert!(!prompt.contains("abcdefghijklmnopqrstuvwxyz0123"), "the key was sent");
    assert_eq!(w.plan().step("b").unwrap().commands[0].text, "Get-Location | Format-List");
    assert_eq!(readiness(&w)["b"], None, "the fix must be validated before it can run");
}

#[test]
fn a_review_adds_concerns_but_changes_no_step() {
    let mut w = validated(&chain());
    let before = w.plan().steps.clone();
    let review = json!({"summary": "Mostly fine.", "findings": [
        {"step": "c", "kind": "missing_validation", "severity": "warning", "message": "Nothing checks c worked."}
    ]});
    let runner = Replay(
        RefCell::new(vec![RunOutput {
            success: true,
            stdout: json!({"type": "result", "is_error": false, "result": format!("```json\n{review}\n```")})
                .to_string(),
            ..RunOutput::default()
        }]),
        RefCell::default(),
    );
    let agents = Agents { runner: &runner, scratch: std::env::temp_dir(), max_repairs: 1 };
    w.review(&agents, &claude(), AT).unwrap();
    assert_eq!(w.plan().steps, before);
    let v = w.view();
    assert_eq!(v.steps.iter().find(|s| s.id == "c").unwrap().concerns, 1);
    assert!(v.notes.iter().any(|n| n.review && n.step.as_deref() == Some("c")));
    assert!(readiness(&w).values().all(|r| *r == Some(Readiness::Ready)), "a review invalidates nothing");
}
