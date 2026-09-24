//! Agent sessions against a fake runner that replays recorded answers, as
//! §56 prescribes for CI: a vendor CLI update must not break the
//! deterministic suite. The shapes of the recorded output follow each CLI's
//! documented format (see `extract.rs`).

#![allow(clippy::unwrap_used)] // Helpers outside #[test] functions may unwrap too.

use std::cell::RefCell;
use std::path::PathBuf;

use keyjutsu_agent::agents::Invocation;
use keyjutsu_agent::context::{ContextItem, prepare};
use keyjutsu_agent::session::{RunOutput, record_review};
use keyjutsu_agent::{AgentError, AgentHandle, AgentKind, Agents, Runner, StepRevision};
use keyjutsu_plan::model::{AgentName, ProvenanceAction};
use serde_json::{Value, json};

const AT: &str = "2026-09-25T03:00:00Z";

/// Replays answers in order and records every invocation.
struct Replay {
    answers: RefCell<Vec<RunOutput>>,
    seen: RefCell<Vec<Invocation>>,
}

impl Replay {
    fn new(answers: Vec<RunOutput>) -> Self {
        Self { answers: RefCell::new(answers.into_iter().rev().collect()), seen: RefCell::new(Vec::new()) }
    }
}

impl Runner for Replay {
    fn run(&self, inv: &Invocation) -> Result<RunOutput, AgentError> {
        self.seen.borrow_mut().push(inv.clone());
        self.answers.borrow_mut().pop().ok_or_else(|| AgentError::Run("no more recorded answers".into()))
    }
}

fn handle(kind: AgentKind) -> AgentHandle {
    AgentHandle { kind, program: PathBuf::from("agent.exe"), version: "9.9.9".into() }
}

fn agents(runner: &Replay) -> Agents<'_, Replay> {
    Agents { runner, scratch: std::env::temp_dir(), max_repairs: 2 }
}

fn plan_doc() -> Value {
    json!({
        "schema_version": "1.0", "plan_id": "p", "task_id": "t",
        "target": {"id": "local", "kind": "local_windows"},
        "agent": {"name": "codex", "version": "0.1"},
        "steps": [
            {"id": "look", "title": "Look", "objective": "See.", "kind": "validation",
             "shell": {"kind": "pwsh"}, "commands": [{"text": "Get-Service -Name docker"}]},
            {"id": "fix", "title": "Fix", "objective": "Repair.", "kind": "command",
             "shell": {"kind": "pwsh"}, "commands": [{"text": "Restart-Service -Name docker"}]}
        ]
    })
}

/// Claude Code's `--output-format json` envelope around a fenced answer.
fn claude_says(doc: &Value) -> RunOutput {
    let message = format!("I checked the service.\n```json\n{}\n```\nRestarting should fix it.", doc);
    RunOutput {
        success: true,
        stdout: json!({"type": "result", "is_error": false, "result": message}).to_string(),
        ..RunOutput::default()
    }
}

fn empty_context() -> keyjutsu_agent::context::PreparedContext {
    prepare(&[]).unwrap()
}

#[test]
fn a_proposal_is_accepted_and_stamped_with_the_agent_that_really_wrote_it() {
    let runner = Replay::new(vec![claude_says(&plan_doc())]);
    let p =
        agents(&runner).propose(&handle(AgentKind::ClaudeCode), "fix docker", &empty_context(), AT).unwrap();

    // The document claimed Codex 0.1; KeyJutsu ran Claude Code 9.9.9.
    assert_eq!(p.plan.plan().agent.name, AgentName::ClaudeCode);
    assert_eq!(p.plan.plan().agent.version, "9.9.9");
    assert_eq!(p.summary, "Restarting should fix it.");
    let provenance = &p.plan.plan().keyjutsu.as_ref().unwrap().provenance;
    assert_eq!(provenance.len(), 2);
    assert!(provenance.iter().all(|e| e.action == ProvenanceAction::Authored));

    let inv = &runner.seen.borrow()[0];
    assert!(inv.args.join(" ").contains("--permission-mode plan"), "investigation is read-only");
    assert!(inv.stdin.contains("fix docker") && inv.stdin.contains("Investigate only"));
}

#[test]
fn an_answer_claiming_keyjutsu_state_is_sent_back_and_then_accepted() {
    let mut cheeky = plan_doc();
    cheeky["keyjutsu"] = json!({"steps": {"fix": {"readiness": "READY", "proof_level": "HIGH"}}});
    let runner = Replay::new(vec![claude_says(&cheeky), claude_says(&plan_doc())]);
    let p =
        agents(&runner).propose(&handle(AgentKind::ClaudeCode), "fix docker", &empty_context(), AT).unwrap();
    assert_eq!(p.attempts, 2);
    let second_prompt = &runner.seen.borrow()[1].stdin;
    assert!(
        second_prompt.contains("could not be accepted") && second_prompt.contains("only by KeyJutsu"),
        "the problem is fed back"
    );
}

#[test]
fn repeated_bad_answers_are_given_up_on() {
    let runner = Replay::new(vec![
        claude_says(&json!({"not": "a plan"})),
        claude_says(&json!({"still": "not"})),
        claude_says(&json!({"nope": true})),
    ]);
    let err = agents(&runner).propose(&handle(AgentKind::ClaudeCode), "t", &empty_context(), AT).unwrap_err();
    assert!(matches!(err, AgentError::Unacceptable { attempts: 3, .. }), "{err}");
}

#[test]
fn an_agent_that_fails_inside_its_envelope_is_reported_not_retried() {
    let runner = Replay::new(vec![RunOutput {
        success: true,
        stdout: json!({"type": "result", "is_error": true, "result": "Not logged in"}).to_string(),
        ..RunOutput::default()
    }]);
    let err = agents(&runner).propose(&handle(AgentKind::ClaudeCode), "t", &empty_context(), AT).unwrap_err();
    assert_eq!(err, AgentError::Failed("Not logged in".into()));
    assert_eq!(runner.seen.borrow().len(), 1);
}

#[test]
fn codexs_answer_is_read_from_its_output_file() {
    let runner = Replay::new(vec![RunOutput {
        success: true,
        stdout: "progress chatter".into(),
        output_file: Some(format!("```json\n{}\n```", plan_doc())),
        ..RunOutput::default()
    }]);
    let p = agents(&runner).propose(&handle(AgentKind::Codex), "t", &empty_context(), AT).unwrap();
    assert_eq!(p.plan.plan().steps.len(), 2);
    let inv = &runner.seen.borrow()[0];
    assert!(inv.args.join(" ").contains("--sandbox read-only"));
    assert!(inv.output_file.is_some());
}

#[test]
fn a_secret_in_pasted_context_never_reaches_the_agent() {
    let context = prepare(&[ContextItem::Text {
        label: "error.log".into(),
        text: "auth failed with token=ghp_0123456789abcdefghijABCDEFGHIJ012345 at 03:00".into(),
    }])
    .unwrap();
    let runner = Replay::new(vec![claude_says(&plan_doc())]);
    agents(&runner).propose(&handle(AgentKind::ClaudeCode), "t", &context, AT).unwrap();
    let prompt = &runner.seen.borrow()[0].stdin;
    assert!(prompt.contains("auth failed"));
    assert!(!prompt.contains("ghp_0123456789"), "the token was sent");
}

#[test]
fn a_step_revision_changes_that_step_only_and_discards_validation() {
    let runner = Replay::new(vec![claude_says(&plan_doc())]);
    let first = agents(&runner).propose(&handle(AgentKind::ClaudeCode), "t", &empty_context(), AT).unwrap();
    let mut validated = first.plan.plan().clone();
    validated.keyjutsu.as_mut().unwrap().steps.insert(
        "fix".into(),
        serde_json::from_value(json!({"readiness": "READY", "proof_level": "MEDIUM"})).unwrap(),
    );

    let renamed = json!({"id": "fixed", "title": "Fix", "objective": "Repair.", "kind": "command",
        "shell": {"kind": "pwsh"}, "commands": [{"text": "Restart-Service -Name docker -Force"}]});
    let good = json!({"id": "fix", "title": "Fix gently", "objective": "Repair.", "kind": "command",
        "shell": {"kind": "pwsh"}, "commands": [{"text": "Start-Service -Name docker"}]});
    let runner = Replay::new(vec![claude_says(&renamed), claude_says(&good)]);
    let findings = vec!["risk: High".to_owned()];
    let request = StepRevision { step: "fix", guidance: "Do not force it.", findings: &findings };
    let revised =
        agents(&runner).revise_step(&handle(AgentKind::ClaudeCode), "t", &validated, &request, AT).unwrap();

    assert_eq!(revised.attempts, 2, "a renamed step is refused and asked for again");
    let p = revised.plan.plan();
    assert_eq!(p.step("fix").unwrap().commands[0].text, "Start-Service -Name docker");
    assert_eq!(p.step("look"), first.plan.plan().step("look"), "the other step is untouched");
    let state = p.keyjutsu.as_ref().unwrap();
    assert!(state.steps.is_empty(), "validation results must not survive a revision");
    let last = state.provenance.last().unwrap();
    assert_eq!((last.step.as_deref(), last.action), (Some("fix"), ProvenanceAction::Revised));
    assert!(last.note.as_deref().unwrap().contains("Do not force it."));
    let prompt = &runner.seen.borrow()[0].stdin;
    assert!(prompt.contains("Revise only step \"fix\"") && prompt.contains("risk: High"));
    assert!(!prompt.contains("\"keyjutsu\":"), "KeyJutsu's state is not shown to the agent");
}

#[test]
fn a_review_challenges_steps_but_changes_nothing() {
    let runner = Replay::new(vec![claude_says(&plan_doc())]);
    let first = agents(&runner).propose(&handle(AgentKind::Codex), "t", &empty_context(), AT).unwrap();
    let before = first.plan.plan().clone();

    let review = json!({"summary": "Risky.", "findings": [
        {"step": "fix", "kind": "weak_rollback", "severity": "warning", "message": "No way back if the restart fails."}
    ]});
    let runner = Replay::new(vec![RunOutput {
        success: true,
        stdout: json!({"response": format!("```json\n{review}\n```")}).to_string(),
        ..RunOutput::default()
    }]);
    let gemini = handle(AgentKind::Gemini);
    let r = agents(&runner).review(&gemini, "t", &before).unwrap();
    assert_eq!(r.findings.len(), 1);
    assert!(runner.seen.borrow()[0].args.join(" ").contains("--approval-mode plan"));

    let recorded = record_review(&before, &gemini, &r, AT);
    assert_eq!(recorded.steps, before.steps, "the review changed no step");
    let last = recorded.keyjutsu.as_ref().unwrap().provenance.last().unwrap();
    assert_eq!(last.action, ProvenanceAction::Challenged);
    assert_eq!(last.actor.agent.as_ref().unwrap().name, AgentName::Gemini);
}
