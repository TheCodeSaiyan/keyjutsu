//! Asking an agent for a plan, a revision or a review, and holding its answer
//! to the same rules as any other untrusted input.
//!
//! - Whatever an agent returns goes through `parse_proposal`: schema,
//!   structure, and the refusal of any KeyJutsu-owned section.
//! - An answer that fails is sent back with the problems, at most
//!   `max_repairs` times; after that the operator sees the failure.
//! - The plan's `agent` field is overwritten with the agent KeyJutsu actually
//!   ran. An agent cannot claim to be another.
//! - A revision of one step may change that step and nothing else, and it
//!   discards earlier validation results: the plan must be validated again.
//! - Every accepted answer is recorded in provenance (§7).

use std::path::PathBuf;
use std::time::Duration;

use keyjutsu_plan::model::{
    Actor, ActorKind, Agent, KeyJutsuState, Plan, ProvenanceAction, ProvenanceEvent, Step,
};
use keyjutsu_plan::{PlanError, ValidPlan, parse_proposal};
use keyjutsu_validation::process;
use serde_json::Value;

use crate::agents::{AgentKind, Invocation, InvocationError, invocation};
use crate::context::PreparedContext;
use crate::extract::{envelope_error, final_message, json_document, summary};
use crate::prompt;
use crate::review::{self, Review};

/// What running an agent once produced.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RunOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub output_file: Option<String>,
}

/// Runs agents. The real one starts processes; tests supply recorded answers.
pub trait Runner {
    fn run(&self, invocation: &Invocation) -> Result<RunOutput, AgentError>;
}

/// Starts the agent's CLI and waits for it, up to `timeout`.
#[derive(Debug, Clone, Copy)]
pub struct ProcessRunner {
    pub timeout: Duration,
}

impl Default for ProcessRunner {
    fn default() -> Self {
        // Agents investigate before answering; ten minutes is generous but finite.
        Self { timeout: Duration::from_secs(600) }
    }
}

impl Runner for ProcessRunner {
    fn run(&self, inv: &Invocation) -> Result<RunOutput, AgentError> {
        let mut command = std::process::Command::new(&inv.program);
        command.args(&inv.args).current_dir(&inv.cwd);
        if let Some(f) = &inv.output_file {
            let _ = std::fs::remove_file(f);
        }
        let done =
            process::run(command, &inv.stdin, self.timeout).map_err(|e| AgentError::Run(e.to_string()))?;
        let output_file = inv.output_file.as_ref().and_then(|f| std::fs::read_to_string(f).ok());
        Ok(RunOutput { success: done.success, stdout: done.stdout, stderr: done.stderr, output_file })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AgentError {
    #[error(transparent)]
    Invocation(#[from] InvocationError),
    #[error("the agent could not be run: {0}")]
    Run(String),
    #[error("the agent failed: {0}")]
    Failed(String),
    #[error("no acceptable answer after {attempts} attempt(s); last problems: {}", problems.join("; "))]
    Unacceptable { attempts: usize, problems: Vec<String> },
    #[error("there is no step `{0}` to revise")]
    UnknownStep(String),
}

/// The agent KeyJutsu is talking to, as detected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentHandle {
    pub kind: AgentKind,
    pub program: PathBuf,
    pub version: String,
}

impl AgentHandle {
    fn identity(&self) -> Agent {
        Agent { name: self.kind.plan_name(), version: self.version.clone() }
    }

    fn actor(&self) -> Actor {
        Actor { kind: ActorKind::Agent, agent: Some(self.identity()) }
    }
}

/// What the operator asks of one step's revision.
#[derive(Debug, Clone, Copy)]
pub struct StepRevision<'a> {
    pub step: &'a str,
    pub guidance: &'a str,
    /// What validation found about the step, so the agent revises against
    /// evidence as well as the operator's words (§11).
    pub findings: &'a [String],
}

/// An accepted plan from an agent, recorded with its provenance.
#[derive(Debug, Clone)]
pub struct Proposal {
    /// A stored plan: the agent's proposal plus KeyJutsu's provenance.
    pub plan: ValidPlan,
    /// What the agent said around its answer.
    pub summary: String,
    pub attempts: usize,
}

pub struct Agents<'r, R: Runner> {
    pub runner: &'r R,
    /// Where agents may write scratch output (Codex's final message).
    pub scratch: PathBuf,
    pub max_repairs: usize,
}

impl<R: Runner> std::fmt::Debug for Agents<'_, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Agents")
            .field("scratch", &self.scratch)
            .field("max_repairs", &self.max_repairs)
            .finish()
    }
}

fn describe(e: &PlanError) -> Vec<String> {
    match e {
        PlanError::Schema { violations } => violations
            .iter()
            .map(|v| format!("{}: {}", if v.at.is_empty() { "(document)" } else { &v.at }, v.message))
            .collect(),
        PlanError::Invalid { problems } => problems.iter().map(ToString::to_string).collect(),
        other => vec![other.to_string()],
    }
}

fn event(
    step: Option<&str>,
    actor: Actor,
    action: ProvenanceAction,
    at: &str,
    note: Option<String>,
) -> ProvenanceEvent {
    ProvenanceEvent { step: step.map(str::to_owned), actor, action, at: at.to_owned(), note }
}

/// The plan without KeyJutsu's section, as an agent is shown it.
fn public_json(plan: &Plan) -> String {
    let mut p = plan.clone();
    p.keyjutsu = None;
    serde_json::to_string_pretty(&p).unwrap_or_default()
}

/// Keep provenance, drop validation results: a revised plan must be
/// validated again before any readiness means anything.
fn carry_provenance(from: &Plan, into: &mut Plan, extra: Vec<ProvenanceEvent>) {
    let mut provenance = from.keyjutsu.as_ref().map(|k| k.provenance.clone()).unwrap_or_default();
    provenance.extend(extra);
    let revision = from.keyjutsu.as_ref().and_then(|k| k.revision).unwrap_or(0) + 1;
    into.keyjutsu = Some(KeyJutsuState {
        revision: Some(revision),
        steps: Default::default(),
        snapshot_hash: None,
        provenance,
    });
}

impl<R: Runner> Agents<'_, R> {
    /// Run `agent` with `prompt` until it gives an answer `accept` takes, or
    /// the repairs run out.
    fn ask<T>(
        &self,
        agent: &AgentHandle,
        prompt: &str,
        cwd: &std::path::Path,
        mut accept: impl FnMut(Value) -> Result<T, Vec<String>>,
    ) -> Result<(T, String, usize), AgentError> {
        let mut current = prompt.to_owned();
        let mut problems = Vec::new();
        for attempt in 1..=self.max_repairs + 1 {
            let inv = invocation(agent.kind, &agent.program, &current, cwd, &self.scratch)?;
            let out = self.runner.run(&inv)?;
            if let Some(why) = envelope_error(&out.stdout) {
                return Err(AgentError::Failed(why));
            }
            if !out.success && out.output_file.is_none() && out.stdout.trim().is_empty() {
                return Err(AgentError::Failed(out.stderr.trim().chars().take(500).collect()));
            }
            let message = final_message(&out.stdout, out.output_file.as_deref());
            problems = match json_document(&message) {
                None => vec!["no JSON document was found in the answer".to_owned()],
                Some(doc) => match accept(doc) {
                    Ok(value) => return Ok((value, summary(&message), attempt)),
                    Err(p) => p,
                },
            };
            current = prompt::repair(prompt, &problems);
        }
        Err(AgentError::Unacceptable { attempts: self.max_repairs + 1, problems })
    }

    fn accept_plan(agent: &AgentHandle, mut doc: Value) -> Result<ValidPlan, Vec<String>> {
        // The agent's identity is KeyJutsu's to state, not the agent's.
        if let Some(obj) = doc.as_object_mut() {
            obj.insert("agent".into(), serde_json::to_value(agent.identity()).unwrap_or(Value::Null));
        }
        parse_proposal(&doc.to_string()).map_err(|e| describe(&e))
    }

    /// Ask `agent` to investigate and propose a plan for `task`.
    pub fn propose(
        &self,
        agent: &AgentHandle,
        task: &str,
        context: &PreparedContext,
        at: &str,
    ) -> Result<Proposal, AgentError> {
        let full = agent.kind.capabilities().prompt_on_stdin;
        let text = prompt::propose(task, context, full);
        let cwd =
            context.working_directory.as_ref().map(PathBuf::from).unwrap_or_else(|| self.scratch.clone());
        let (plan, summary, attempts) = self.ask(agent, &text, &cwd, |doc| Self::accept_plan(agent, doc))?;
        let mut stored = plan.plan().clone();
        let authored = stored
            .steps
            .iter()
            .map(|s| event(Some(&s.id), agent.actor(), ProvenanceAction::Authored, at, None))
            .collect();
        carry_provenance(&Plan { keyjutsu: None, ..stored.clone() }, &mut stored, authored);
        let plan = ValidPlan::revalidate(stored, false)
            .map_err(|e| AgentError::Unacceptable { attempts, problems: describe(&e) })?;
        Ok(Proposal { plan, summary, attempts })
    }

    /// Ask `agent` to revise one step, with the operator's guidance and what
    /// validation found. Only that step may change.
    pub fn revise_step(
        &self,
        agent: &AgentHandle,
        task: &str,
        plan: &Plan,
        request: &StepRevision<'_>,
        at: &str,
    ) -> Result<Proposal, AgentError> {
        let StepRevision { step, guidance, findings } = *request;
        let index = plan
            .steps
            .iter()
            .position(|s| s.id == step)
            .ok_or_else(|| AgentError::UnknownStep(step.to_owned()))?;
        let text = prompt::revise_step(task, &public_json(plan), step, guidance, findings);
        let base = {
            let mut p = plan.clone();
            p.keyjutsu = None;
            p
        };
        let (revised, summary, attempts) = self.ask(agent, &text, &self.scratch, |doc| {
            // Accept either the step itself or a document wrapping it.
            let candidate = doc.get("step").cloned().unwrap_or(doc);
            let replacement: Step =
                serde_json::from_value(candidate).map_err(|e| vec![format!("not a step: {e}")])?;
            if replacement.id != step {
                return Err(vec![format!("the step's id must stay \"{step}\", not \"{}\"", replacement.id)]);
            }
            let mut candidate_plan = base.clone();
            candidate_plan.steps[index] = replacement;
            let doc = serde_json::to_value(&candidate_plan).map_err(|e| vec![e.to_string()])?;
            Self::accept_plan(agent, doc)
        })?;
        let mut stored = revised.plan().clone();
        let note = (!guidance.trim().is_empty())
            .then(|| format!("guidance: {}", guidance.chars().take(500).collect::<String>()));
        carry_provenance(
            plan,
            &mut stored,
            vec![event(Some(step), agent.actor(), ProvenanceAction::Revised, at, note)],
        );
        let plan = ValidPlan::revalidate(stored, false)
            .map_err(|e| AgentError::Unacceptable { attempts, problems: describe(&e) })?;
        Ok(Proposal { plan, summary, attempts })
    }

    /// Ask `agent` to reconsider the whole plan.
    pub fn revise_plan(
        &self,
        agent: &AgentHandle,
        task: &str,
        plan: &Plan,
        guidance: &str,
        at: &str,
    ) -> Result<Proposal, AgentError> {
        let full = agent.kind.capabilities().prompt_on_stdin;
        let text = prompt::revise_plan(task, &public_json(plan), guidance, full);
        let (revised, summary, attempts) =
            self.ask(agent, &text, &self.scratch, |doc| Self::accept_plan(agent, doc))?;
        let mut stored = revised.plan().clone();
        let changed: Vec<ProvenanceEvent> = stored
            .steps
            .iter()
            .filter(|s| plan.step(&s.id) != Some(*s))
            .map(|s| {
                let action = if plan.step(&s.id).is_some() {
                    ProvenanceAction::Revised
                } else {
                    ProvenanceAction::Authored
                };
                event(Some(&s.id), agent.actor(), action, at, None)
            })
            .collect();
        carry_provenance(plan, &mut stored, changed);
        let plan = ValidPlan::revalidate(stored, false)
            .map_err(|e| AgentError::Unacceptable { attempts, problems: describe(&e) })?;
        Ok(Proposal { plan, summary, attempts })
    }

    /// Ask a second agent to challenge the plan. It cannot change anything.
    pub fn review(&self, reviewer: &AgentHandle, task: &str, plan: &Plan) -> Result<Review, AgentError> {
        let ids: Vec<&str> = plan.steps.iter().map(|s| s.id.as_str()).collect();
        let text = prompt::review(task, &public_json(plan));
        let (r, _, _) = self.ask(reviewer, &text, &self.scratch, |doc| {
            review::parse(doc, &ids).map_err(|e| vec![e.to_string()])
        })?;
        Ok(r)
    }
}

/// Record a review in the plan's provenance: the reviewer challenged each
/// step it raised something about.
pub fn record_review(plan: &Plan, reviewer: &AgentHandle, review: &Review, at: &str) -> Plan {
    let mut out = plan.clone();
    let mut state = out.keyjutsu.take().unwrap_or(KeyJutsuState {
        revision: None,
        steps: Default::default(),
        snapshot_hash: None,
        provenance: Vec::new(),
    });
    for f in &review.findings {
        state.provenance.push(event(
            f.step.as_deref(),
            reviewer.actor(),
            ProvenanceAction::Challenged,
            at,
            Some(format!(
                "{:?} ({:?}): {}",
                f.kind,
                f.severity,
                f.message.chars().take(300).collect::<String>()
            )),
        ));
    }
    out.keyjutsu = Some(state);
    out
}
