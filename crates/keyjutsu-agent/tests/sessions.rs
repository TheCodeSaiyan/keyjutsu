//! Agent sessions against a fake runner that replays recorded answers, so
//! that a vendor CLI update cannot break the deterministic suite CI runs. The
//! shapes of the recorded output follow each CLI's documented format (see
//! `extract.rs`).

#![allow(clippy::unwrap_used)] // Helpers outside #[test] functions may unwrap too.

use std::cell::RefCell;
use std::path::PathBuf;

use keyjutsu_agent::agents::Invocation;
use keyjutsu_agent::context::{ContextItem, prepare};
use keyjutsu_agent::session::{RunOutput, record_review};
use keyjutsu_agent::{AgentError, AgentHandle, AgentKind, Agents, Runner, StepRevision};
use keyjutsu_plan::model::{AgentName, ExecutionMode, ProvenanceAction};
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

/// Secret leakage, by every route: not only pasted context. The task, the
/// guidance for a revision, what validation found, and a value the operator
/// typed into a step are all redacted before any agent sees them.
#[test]
fn no_secret_reaches_an_agent_by_any_route() {
    const TOKEN: &str = "ghp_0123456789abcdefghijABCDEFGHIJ012345";
    let task = format!("Fix the build; it fails with token={TOKEN}");
    let runner = Replay::new(vec![claude_says(&plan_doc())]);
    let first = agents(&runner).propose(&handle(AgentKind::ClaudeCode), &task, &empty_context(), AT).unwrap();
    let proposal = runner.seen.borrow()[0].stdin.clone();

    let mut edited = first.plan.plan().clone();
    edited.steps[0].commands[0].text = format!("gh auth login --with-token {TOKEN}");
    let guidance = format!("Use my token {TOKEN} for it");
    let findings = vec![format!("dry run printed: Authorization: Bearer {}", &TOKEN[4..])];
    let request = StepRevision { step: "fix", guidance: &guidance, findings: &findings, failure: None };
    let step = json!({"id": "fix", "title": "Fix", "objective": "Repair.", "kind": "command",
        "shell": {"kind": "pwsh"}, "commands": [{"text": "Start-Service -Name docker"}]});
    let runner = Replay::new(vec![
        claude_says(&step),
        claude_says(&plan_doc()),
        claude_says(&json!({"summary": "fine", "findings": []})),
    ]);
    let a = agents(&runner);
    a.revise_step(&handle(AgentKind::ClaudeCode), &task, &edited, &request, AT).unwrap();
    a.revise_plan(&handle(AgentKind::ClaudeCode), &task, &edited, &guidance, AT).unwrap();
    a.review(&handle(AgentKind::ClaudeCode), &task, &edited).unwrap();

    let mut prompts = vec![proposal];
    prompts.extend(runner.seen.borrow().iter().map(|c| c.stdin.clone()));
    assert_eq!(prompts.len(), 4, "a revision, a whole-plan revision and a review, and the proposal");
    for prompt in &prompts {
        assert!(!prompt.contains(&TOKEN[4..20]), "the token was sent:\n{prompt}");
        assert!(prompt.contains("[REDACTED"), "{prompt}");
    }
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
    let request =
        StepRevision { step: "fix", guidance: "Do not force it.", findings: &findings, failure: None };
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

/// Found by the first live check: Gemini CLI 0.32.1 without
/// `experimental.plan` warns and falls back to its default mode.
#[test]
fn an_answer_from_outside_the_read_only_mode_is_discarded() {
    let warning = "Approval mode \"plan\" is only available when experimental.plan is enabled. Falling back to \"default\".";
    let runner = Replay::new(vec![RunOutput {
        success: true,
        stdout: json!({"response": format!("```json\n{}\n```", plan_doc())}).to_string(),
        stderr: warning.into(),
        ..RunOutput::default()
    }]);
    let err = agents(&runner).propose(&handle(AgentKind::Gemini), "t", &empty_context(), AT).unwrap_err();
    assert!(matches!(&err, AgentError::NotReadOnly(why) if why.contains("experimental")), "{err}");
    assert_eq!(runner.seen.borrow().len(), 1, "not asked again in the same mode");
}

/// ADR 0020 phase 2: the agent is told it may ask, and a proposal that asks
/// keeps its questions. One whose question hides a character is sent back,
/// like any other refusal.
#[test]
fn an_agent_may_ask_instead_of_guessing() {
    let mut asking = plan_doc();
    asking["questions"] = json!([
        {"id": "restart-or-reinstall", "step": "fix", "text": "Restart the service, or reinstall Docker Desktop?",
         "options": ["Restart it", "Reinstall"]}
    ]);
    let mut hidden = asking.clone();
    hidden["questions"][0]["options"][1] = json!("Reinstall\u{202E}");
    let runner = Replay::new(vec![claude_says(&hidden), claude_says(&asking)]);
    let p =
        agents(&runner).propose(&handle(AgentKind::ClaudeCode), "fix docker", &empty_context(), AT).unwrap();

    assert_eq!(p.attempts, 2);
    let q = &p.plan.plan().questions;
    assert_eq!(q.len(), 1);
    assert_eq!((q[0].id.as_str(), q[0].step.as_deref()), ("restart-or-reinstall", Some("fix")));
    assert_eq!(q[0].options, ["Restart it", "Reinstall"]);
    let seen = runner.seen.borrow();
    assert!(seen[0].stdin.contains("ask rather than guess"), "the agent is told it may ask");
    assert!(seen[1].stdin.contains("U+202E"), "the hidden character is named when it is sent back");
}

/// How a plan runs is the operator's choice. An agent that writes a mode on
/// every step (Claude Code writes "assisted") would otherwise override the
/// run's mode step by step, so choosing Auto changed nothing. Only a
/// credential step keeps its mode, because typing it yourself is what it is.
#[test]
fn how_a_step_runs_is_not_the_agents_to_choose() {
    let mut doc = plan_doc();
    doc["steps"][0]["execution_mode"] = json!("assisted");
    doc["steps"][1]["execution_mode"] = json!("direct");
    doc["steps"].as_array_mut().unwrap().push(json!({
        "id": "token", "title": "Token", "objective": "Ask for it.", "kind": "credential",
        "execution_mode": "user_input",
        "credential": {"variable": "TOKEN", "prompt": "Token", "kind": "secret"}
    }));
    let runner = Replay::new(vec![claude_says(&doc)]);
    let p =
        agents(&runner).propose(&handle(AgentKind::ClaudeCode), "fix docker", &empty_context(), AT).unwrap();
    let modes: Vec<Option<ExecutionMode>> = p.plan.plan().steps.iter().map(|s| s.execution_mode).collect();
    assert_eq!(modes, [None, None, Some(ExecutionMode::UserInput)]);
}

/// A mode the operator gave a step stays through the agent's revision of
/// the plan; one the agent adds is dropped.
#[test]
fn a_mode_the_operator_chose_survives_a_revision() {
    let mut mine = plan_doc();
    mine["steps"][0]["execution_mode"] = json!("direct");
    let before: keyjutsu_plan::model::Plan = serde_json::from_value(mine.clone()).unwrap();
    let mut back = mine.clone();
    back["steps"][1]["execution_mode"] = json!("assisted");
    let runner = Replay::new(vec![claude_says(&back)]);
    let p =
        agents(&runner).revise_plan(&handle(AgentKind::ClaudeCode), "fix docker", &before, "g", AT).unwrap();
    let modes: Vec<Option<ExecutionMode>> = p.plan.plan().steps.iter().map(|s| s.execution_mode).collect();
    assert_eq!(modes, [Some(ExecutionMode::Direct), None]);
}

/// A slow stand-in for an agent's CLI: `cmd` waiting half a minute.
fn slow_agent() -> Invocation {
    Invocation {
        program: PathBuf::from("cmd"),
        args: vec!["/D".into(), "/C".into(), "ping -n 30 127.0.0.1 >NUL".into()],
        stdin: String::new(),
        cwd: std::env::temp_dir(),
        output_file: None,
    }
}

#[test]
fn an_agent_the_operator_stops_ends_saying_so() {
    let runner = keyjutsu_agent::ProcessRunner::default();
    runner.stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let started = std::time::Instant::now();
    let e = runner.run(&slow_agent()).unwrap_err();
    assert_eq!(e, AgentError::Stopped);
    assert_eq!(e.to_string(), "stopped at your request; nothing was changed");
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}

#[test]
fn an_agent_that_overruns_says_how_long_it_had() {
    let runner = keyjutsu_agent::ProcessRunner {
        timeout: std::time::Duration::from_millis(500),
        ..keyjutsu_agent::ProcessRunner::default()
    };
    let e = runner.run(&slow_agent()).unwrap_err();
    assert_eq!(e, AgentError::TimedOut { minutes: 1 });
    assert!(e.to_string().starts_with("the agent had not answered after 1 minute,"), "{e}");
}
