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
use keyjutsu_core::asks::{AskFrom, Choice};
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

/// A plan saved from the workspace keeps what validation found for every
/// step, and opens again as it was: something to hand over when a plan will
/// not go, or to come back to.
#[test]
fn a_saved_plan_keeps_its_findings_and_opens_again_as_it_was() {
    let w = validated(&chain());
    let dir = std::env::temp_dir().join(format!("kj-saved-plan-{}", std::process::id()));
    let file = w.save_draft(&dir).unwrap();
    assert!(file.file_name().unwrap().to_string_lossy().starts_with("p-"), "{}", file.display());
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("\"evidence\""), "the findings are in the file");
    let reopened = Workspace::open(&text).unwrap();
    assert_eq!(reopened.plan(), w.plan(), "the same plan, findings and all");
    assert_eq!(readiness(&reopened), readiness(&w));
    let _ = std::fs::remove_dir_all(&dir);
}

fn reviewed(concern: &str) -> Workspace {
    let mut w = validated(&chain());
    let review = json!({"summary": "Mostly fine.", "findings": [
        {"step": "c", "kind": "missing_validation", "severity": "warning", "message": concern}
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
    w
}

/// ADR 0020: a reviewer's concern is something to answer. Dismissing it
/// takes it out of what needs the operator and keeps why, in the plan.
#[test]
fn a_dismissed_concern_leaves_what_needs_you_and_is_kept_in_the_provenance() {
    let mut w = reviewed("Nothing checks c worked.");
    let asks = w.view().asks;
    let ask = asks.iter().find(|a| a.step.as_deref() == Some("c")).unwrap();
    assert!(matches!(&ask.from, AskFrom::Review { .. }), "{ask:?}");
    let Some(Choice::Dismiss { note }) = ask.choices.iter().find(|c| matches!(c, Choice::Dismiss { .. }))
    else {
        panic!("no way to dismiss it: {ask:?}");
    };
    assert!(ask.choices.contains(&Choice::EditStep));

    w.dismiss(*note, "Get-Host cannot fail here.", AT).unwrap();

    assert!(w.view().asks.iter().all(|a| a.step.as_deref() != Some("c")), "{:?}", w.view().asks);
    let last = w.plan().keyjutsu.as_ref().unwrap().provenance.last().unwrap();
    assert_eq!((last.step.as_deref(), last.action), (Some("c"), ProvenanceAction::Dismissed));
    let said = last.note.as_deref().unwrap();
    assert!(
        said.contains("Nothing checks c worked.") && said.contains("Get-Host cannot fail here."),
        "{said}"
    );
    assert!(readiness(&w).values().all(|r| *r == Some(Readiness::Ready)), "dismissing invalidates nothing");
    assert!(w.dismiss(*note, "again", AT).is_err(), "a concern is dismissed once");
    assert!(w.dismiss(0, "not a concern", AT).is_err(), "the operator's own note is not a concern");
    assert!(
        parse_plan(&serde_json::to_string(w.plan()).unwrap()).is_ok(),
        "the plan still matches its schema"
    );
}

/// A concern and a reason are each allowed up to the plan's note length, so
/// the two together are clipped rather than make the plan invalid.
#[test]
fn dismissing_a_long_concern_keeps_the_plan_valid() {
    let long = "x".repeat(3900);
    let mut w = reviewed(&long);
    let note = w.view().notes.iter().position(|n| n.review).unwrap();
    w.dismiss(note, &"y".repeat(3900), AT).unwrap();
    assert!(parse_plan(&serde_json::to_string(w.plan()).unwrap()).is_ok());
}

fn understated() -> Value {
    json!({
        "schema_version": "1.0", "plan_id": "p", "task_id": "t",
        "target": {"id": "local", "kind": "local_windows"},
        "agent": {"name": "codex", "version": "1"},
        "steps": [
            {"id": "wipe", "title": "Remove scratch data", "objective": "Start clean.", "kind": "command",
             "shell": {"kind": "pwsh"},
             "proposed_risk": {"level": "low", "rationale": "Only scratch."},
             "commands": [{"text": "Remove-Item -Recurse -Force -LiteralPath C:/KeyJutsu-does-not-exist/scratch"}]}
        ]
    })
}

/// The agent rated a step lower than KeyJutsu does: the ask offers
/// KeyJutsu's rating, and taking it sends the step back to validation, where
/// it no longer needs review for that.
#[test]
fn an_understated_risk_can_be_answered_by_taking_keyjutsus_rating() {
    let mut w = validated(&understated());
    let ask = w.view().asks.into_iter().find(|a| a.step.as_deref() == Some("wipe")).unwrap();
    assert_eq!(ask.from, AskFrom::Validation { check: "risk".into() }, "{ask:?}");
    assert_eq!(ask.choices[0], Choice::UseKeyJutsuRating);

    let change = w.use_keyjutsu_rating("wipe", AT).unwrap();
    assert_eq!(change.affected, ["wipe"]);
    let risk = w.plan().step("wipe").unwrap().proposed_risk.clone().unwrap();
    assert!(risk.rationale.starts_with("KeyJutsu's rating"), "{}", risk.rationale);
    w.validate(Options { dry_run: false, ..Options::default() }, AT).unwrap();
    assert!(
        w.view().asks.iter().all(|a| a.from != AskFrom::Validation { check: "risk".into() }),
        "{:?}",
        w.view().asks
    );
    assert!(w.use_keyjutsu_rating("nowhere", AT).is_err());
}

/// `chain()`, with the author asking which service step b should look at.
fn asking() -> Value {
    let mut v = chain();
    v["questions"] = json!([
        {"id": "which-service", "step": "b", "text": "Which service should b look at?",
         "options": ["Winmgmt", "Spooler"], "free_text": true}
    ]);
    v
}

/// ADR 0020 phase 2: answering sends the question and the answer to the
/// agent, and what comes back is adopted like any revision: unvalidated,
/// with the question closed even if the agent asks it again.
#[test]
fn answering_a_question_asks_the_agent_and_closes_it() {
    let mut w = validated(&asking());
    let ask = w.view().asks.into_iter().find(|a| matches!(a.from, AskFrom::Agent { .. })).unwrap();
    assert_eq!(ask.step.as_deref(), Some("b"));
    assert!(
        ask.choices.contains(&Choice::Answer { question: "which-service".into(), answer: "Spooler".into() })
    );

    let mut revised = asking(); // the agent asks again: it is closed all the same
    revised["steps"][1]["commands"][0]["text"] = json!("Get-Service -Name Spooler");
    let runner = Replay(RefCell::new(vec![claude_says(&revised, "Looking at Spooler.")]), RefCell::default());
    let agents = Agents { runner: &runner, scratch: std::env::temp_dir(), max_repairs: 1 };
    let change = w.answer(&agents, &claude(), "which-service", "Spooler", AT).unwrap();

    let prompt = &runner.1.borrow()[0].stdin;
    assert!(prompt.contains("You asked: Which service should b look at?"), "{prompt}");
    assert!(prompt.contains("The operator answered: Spooler"), "{prompt}");
    assert_eq!(w.plan().step("b").unwrap().commands[0].text, "Get-Service -Name Spooler");
    assert!(w.plan().questions.is_empty(), "the question is closed");
    assert!(change.affected.contains(&"b".to_owned()));
    assert_eq!(readiness(&w)["b"], None, "the revision is not validated");
    let events = &w.plan().keyjutsu.as_ref().unwrap().provenance;
    let answered = events.iter().find(|e| e.action == ProvenanceAction::Answered).unwrap();
    assert_eq!(answered.step.as_deref(), Some("b"));
    let said = answered.note.as_deref().unwrap();
    assert!(said.contains("Which service should b look at?") && said.contains("Spooler"), "{said}");
    assert!(w.view().asks.iter().all(|a| !matches!(a.from, AskFrom::Agent { .. })));
    assert!(w.answer(&agents, &claude(), "which-service", "Winmgmt", AT).is_err(), "answered once");
}

/// Carrying on asks the agent nothing and changes no step, so validation
/// stands; the decision is still kept.
#[test]
fn carrying_on_closes_a_question_and_keeps_validation() {
    let mut w = validated(&asking());
    let before = w.plan().steps.clone();
    w.carry_on("which-service", AT).unwrap();
    assert!(w.plan().questions.is_empty());
    assert_eq!(w.plan().steps, before);
    assert!(readiness(&w).values().all(|r| *r == Some(Readiness::Ready)), "{:?}", readiness(&w));
    let last = w.plan().keyjutsu.as_ref().unwrap().provenance.last().unwrap();
    assert_eq!((last.step.as_deref(), last.action), (Some("b"), ProvenanceAction::Answered));
    assert!(last.note.as_deref().unwrap().contains("carry on as planned"));
    assert!(w.carry_on("which-service", AT).is_err(), "closed once");
    assert!(w.carry_on("nothing-asked", AT).is_err());
}

/// An answer in the operator's own words may be long; the record of it
/// still fits the plan's note, so the plan stays valid.
#[test]
fn a_long_answer_keeps_the_plan_valid() {
    let mut w = validated(&asking());
    let runner = Replay(RefCell::new(vec![claude_says(&chain(), "Done.")]), RefCell::default());
    let agents = Agents { runner: &runner, scratch: std::env::temp_dir(), max_repairs: 1 };
    w.answer(&agents, &claude(), "which-service", &"z".repeat(5000), AT).unwrap();
    assert!(parse_plan(&serde_json::to_string(w.plan()).unwrap()).is_ok());
}

/// An empty answer is not sent: the agent would be told nothing.
#[test]
fn an_empty_answer_is_not_sent() {
    let mut w = validated(&asking());
    let runner = Replay(RefCell::new(vec![claude_says(&chain(), "Done.")]), RefCell::default());
    let agents = Agents { runner: &runner, scratch: std::env::temp_dir(), max_repairs: 1 };
    assert!(w.answer(&agents, &claude(), "which-service", "  ", AT).is_err());
    assert!(runner.1.borrow().is_empty(), "the agent was asked");
    assert_eq!(w.plan().questions.len(), 1, "the question is still open");
}

/// An open question is not a reason to refuse approval: what the plan does
/// is judged by validation alone.
#[test]
fn an_unanswered_question_does_not_block_approval() {
    let w = validated(&asking());
    assert!(w.view().overall.blocking.is_empty(), "{:?}", w.view().overall.blocking);
    let snap = w.approve(&BTreeMap::new(), None, AT).unwrap();
    assert_eq!(snap.plan().questions.len(), 1);
}
