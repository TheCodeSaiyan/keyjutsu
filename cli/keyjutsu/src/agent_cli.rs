//! `keyjutsu agents` and `keyjutsu plan propose | revise | review`.
//!
//! Nothing is sent to an agent without `--send`. Without it, KeyJutsu prints
//! what would go (the context manifest, with redactions and any
//! secret-looking files in a folder) and stops, so the operator always sees
//! the scope of what leaves the machine first.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use keyjutsu_core::agent::context::{ContextItem, PreparedContext, prepare};
use keyjutsu_core::agent::detect::{SignIn, detect, detect_all};
use keyjutsu_core::agent::session::record_review;
use keyjutsu_core::agent::{AgentHandle, AgentKind, Agents, ProcessRunner, StepRevision};
use keyjutsu_core::fingerprint;
use keyjutsu_core::plan::model::EvidenceResult;
use keyjutsu_core::plan::{Plan, parse_plan};
use keyjutsu_core::validation::{self, Options};

fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join("keyjutsu-agent-scratch");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn list(json: bool) -> ExitCode {
    let agents = detect_all();
    if json {
        println!("{}", serde_json::to_string_pretty(&agents).unwrap_or_default());
        return ExitCode::SUCCESS;
    }
    for a in &agents {
        let status = match (&a.path, &a.version) {
            (None, _) => "not installed".to_owned(),
            (Some(_), Some(v)) => format!("version {v}"),
            (Some(_), None) => "installed, version unknown".to_owned(),
        };
        let sign_in = match a.sign_in {
            SignIn::CredentialsFound => "credentials found",
            SignIn::NoCredentialsFound => "no credentials found",
            SignIn::Unknown => "sign-in unknown",
        };
        println!("  {:<20} {:<28} {}", a.name, status, if a.installed() { sign_in } else { "" });
        if !a.installed() {
            println!("      get it: {}", a.install_url);
        }
        if a.installed() {
            println!("      read-only: {}", a.capabilities.read_only);
            if a.needs_compatibility_check {
                println!(
                    "      ! adapter checked against {}; run `keyjutsu agents check --live`",
                    a.capabilities.verified_with.unwrap_or("no version")
                );
            }
        }
    }
    ExitCode::SUCCESS
}

fn handle(name: &str) -> Result<AgentHandle, ExitCode> {
    let Some(kind) = AgentKind::from_name(name) else {
        eprintln!("keyjutsu: unknown agent `{name}`; choose codex, claude, gemini, copilot or cursor");
        return Err(ExitCode::from(2));
    };
    let info = detect(kind);
    match (info.path, info.version) {
        (Some(path), version) => Ok(AgentHandle {
            kind,
            program: PathBuf::from(path),
            version: version.unwrap_or_else(|| "unknown".into()),
        }),
        (None, _) => {
            eprintln!(
                "keyjutsu: {} is not installed (looked for `{}` on PATH)",
                kind.display_name(),
                kind.executable()
            );
            Err(ExitCode::FAILURE)
        }
    }
}

fn show_manifest(agent: &AgentHandle, context: &PreparedContext) {
    println!("Request for {} {}, in its read-only mode:", agent.kind.display_name(), agent.version);
    println!("  {}", agent.kind.capabilities().read_only);
    if context.manifest.is_empty() {
        println!("  context: none beyond the task");
    }
    for m in &context.manifest {
        match m.kind.as_str() {
            "folder" => {
                println!(
                    "  folder   {} (the agent investigates it itself; KeyJutsu cannot redact what it reads)",
                    m.label
                );
                for f in &m.sensitive_files {
                    println!("           ! looks sensitive: {f}");
                }
            }
            kind => {
                let trunc = if m.truncated { ", truncated" } else { "" };
                println!("  {kind:<8} {} ({} characters{trunc})", m.label, m.chars_sent);
                for r in &m.redactions {
                    println!("           redacted: {r}");
                }
            }
        }
    }
}

fn write_plan(out: &Path, plan: &Plan) -> ExitCode {
    match std::fs::write(out, serde_json::to_string_pretty(plan).unwrap_or_default()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("keyjutsu: cannot write {}: {e}", out.display());
            ExitCode::FAILURE
        }
    }
}

pub fn propose(
    task: &str,
    agent: &str,
    send: bool,
    files: &[PathBuf],
    folder: Option<&Path>,
    out: &Path,
) -> ExitCode {
    let agent = match handle(agent) {
        Ok(a) => a,
        Err(c) => return c,
    };
    let mut items: Vec<ContextItem> = files.iter().cloned().map(ContextItem::File).collect();
    if let Some(f) = folder {
        items.push(ContextItem::Folder(f.to_path_buf()));
    }
    let context = match prepare(&items) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("keyjutsu: {e}");
            return ExitCode::from(2);
        }
    };
    show_manifest(&agent, &context);
    if !send {
        println!();
        println!("Nothing was sent. Add --send to send this request.");
        return ExitCode::SUCCESS;
    }
    let runner = ProcessRunner::default();
    let agents = Agents { runner: &runner, scratch: scratch(), max_repairs: 2 };
    println!();
    println!("Waiting for {}…", agent.kind.display_name());
    match agents.propose(&agent, task, &context, &fingerprint::now_rfc3339()) {
        Ok(p) => {
            if !p.summary.is_empty() {
                println!();
                println!("{}", p.summary);
            }
            println!();
            println!(
                "Proposed {} in {}; written to {}.",
                crate::count(p.plan.plan().steps.len(), "step", "steps"),
                crate::count(p.attempts, "attempt", "attempts"),
                out.display()
            );
            println!("Next: keyjutsu plan validate {}", out.display());
            write_plan(out, p.plan.plan())
        }
        Err(e) => {
            eprintln!("keyjutsu: {e}");
            ExitCode::FAILURE
        }
    }
}

fn load(file: &Path) -> Result<Plan, ExitCode> {
    let text = std::fs::read_to_string(file).map_err(|e| {
        eprintln!("keyjutsu: cannot read {}: {e}", file.display());
        ExitCode::from(2)
    })?;
    parse_plan(&text).map(|p| p.into_plan()).map_err(|e| {
        eprintln!("{}: {e}", file.display());
        ExitCode::FAILURE
    })
}

pub struct Revise<'a> {
    pub file: &'a Path,
    pub step: &'a str,
    pub guidance: &'a str,
    pub task: Option<&'a str>,
    pub session: Option<&'a str>,
    pub agent: &'a str,
    pub send: bool,
    pub out: &'a Path,
}

/// The failure of `step` in recorded session `id`, for the agent to read.
fn recorded_failure(id: &str, step: &str) -> Result<keyjutsu_core::agent::RunFailure, String> {
    let store = keyjutsu_core::store::Store::open(&keyjutsu_core::store::default_root())?;
    let record = keyjutsu_core::history::load(&store, id)?;
    match record.outcome {
        keyjutsu_core::execute::Outcome::Failed { step: failed, expected, actual, output }
            if failed == step =>
        {
            Ok(keyjutsu_core::agent::RunFailure { expected, actual, output })
        }
        keyjutsu_core::execute::Outcome::Failed { step: failed, .. } => {
            Err(format!("in session {id} it was step `{failed}` that failed, not `{step}`"))
        }
        _ => Err(format!("session {id} did not end in a failed step")),
    }
}

pub fn revise(args: Revise<'_>) -> ExitCode {
    let Revise { file, step, guidance, task, session, agent, send, out } = args;
    let failure = match session.map(|id| recorded_failure(id, step)).transpose() {
        Ok(f) => f,
        Err(e) => {
            eprintln!("keyjutsu: {e}");
            return ExitCode::FAILURE;
        }
    };
    let plan = match load(file) {
        Ok(p) => p,
        Err(c) => return c,
    };
    let agent = match handle(agent) {
        Ok(a) => a,
        Err(c) => return c,
    };
    let task = task.map(str::to_owned).or_else(|| plan.title.clone()).unwrap_or_else(|| plan.task_id.clone());
    // What validation says about the step now, so the agent revises against
    // evidence rather than only the operator's words.
    let findings: Vec<String> = match keyjutsu_core::plan::ValidPlan::revalidate(plan.clone(), false) {
        Ok(valid) => validation::validate(&valid, Options { dry_run: false, ..Options::default() })
            .steps
            .get(step)
            .map(|s| {
                s.evidence
                    .iter()
                    .filter(|e| e.result == EvidenceResult::Failed)
                    .map(|e| format!("{}: {}", e.check, e.detail.clone().unwrap_or_default()))
                    .collect()
            })
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    println!("Revising step `{step}` with {} {}.", agent.kind.display_name(), agent.version);
    println!("  guidance: {guidance}");
    for f in &findings {
        println!("  validation: {f}");
    }
    if let Some(f) = &failure {
        println!("  failed:     {} ({})", f.actual, f.expected);
        println!("  output:     the last {} characters it printed, redacted", f.output.chars().count());
    }
    if !send {
        println!();
        println!("Nothing was sent. Add --send to send this request.");
        return ExitCode::SUCCESS;
    }
    let runner = ProcessRunner::default();
    let agents = Agents { runner: &runner, scratch: scratch(), max_repairs: 2 };
    match agents.revise_step(
        &agent,
        &task,
        &plan,
        &StepRevision { step, guidance, findings: &findings, failure: failure.as_ref() },
        &fingerprint::now_rfc3339(),
    ) {
        Ok(p) => {
            println!("Revised; written to {}. Validate it again before approving.", out.display());
            write_plan(out, p.plan.plan())
        }
        Err(e) => {
            eprintln!("keyjutsu: {e}");
            ExitCode::FAILURE
        }
    }
}

pub fn review(file: &Path, task: Option<&str>, agent: &str, send: bool, record: Option<&Path>) -> ExitCode {
    let plan = match load(file) {
        Ok(p) => p,
        Err(c) => return c,
    };
    let reviewer = match handle(agent) {
        Ok(a) => a,
        Err(c) => return c,
    };
    let task = task.map(str::to_owned).or_else(|| plan.title.clone()).unwrap_or_else(|| plan.task_id.clone());
    println!(
        "Review by {} {}: it can challenge the plan but not change it.",
        reviewer.kind.display_name(),
        reviewer.version
    );
    if !send {
        println!();
        println!("Nothing was sent. Add --send to send this request.");
        return ExitCode::SUCCESS;
    }
    let runner = ProcessRunner::default();
    let agents = Agents { runner: &runner, scratch: scratch(), max_repairs: 2 };
    match agents.review(&reviewer, &task, &plan) {
        Ok(r) => {
            if !r.summary.is_empty() {
                println!("{}", r.summary);
            }
            for f in &r.findings {
                println!(
                    "  {:<8} {:<18} {}: {}",
                    format!("{:?}", f.severity),
                    format!("{:?}", f.kind),
                    f.step.as_deref().unwrap_or("(plan)"),
                    f.message
                );
            }
            if r.findings.is_empty() {
                println!("  No findings.");
            }
            match record {
                Some(out) => {
                    write_plan(out, &record_review(&plan, &reviewer, &r, &fingerprint::now_rfc3339()))
                }
                None => ExitCode::SUCCESS,
            }
        }
        Err(e) => {
            eprintln!("keyjutsu: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The live compatibility check: a tiny request to each installed
/// agent, to confirm its adapter still works with the installed version.
pub fn check(live: bool) -> ExitCode {
    if !live {
        eprintln!("keyjutsu: this sends a small request to each installed agent on your accounts.");
        eprintln!("          Run `keyjutsu agents check --live` to do it.");
        return ExitCode::from(2);
    }
    let runner = ProcessRunner { timeout: std::time::Duration::from_secs(180) };
    let agents = Agents { runner: &runner, scratch: scratch(), max_repairs: 0 };
    let mut failures = 0;
    for info in detect_all().into_iter().filter(|a| a.installed()) {
        let handle = AgentHandle {
            kind: info.kind,
            program: PathBuf::from(info.path.clone().unwrap_or_default()),
            version: info.version.clone().unwrap_or_else(|| "unknown".into()),
        };
        let context = PreparedContext { manifest: Vec::new(), working_directory: None, blocks: Vec::new() };
        let task = "Compatibility check. Propose a one-step plan whose single validation step runs `Get-Date` in pwsh.";
        let result = agents.propose(&handle, task, &context, &fingerprint::now_rfc3339());
        match result {
            Ok(p) => println!(
                "  ok     {:<20} {} ({})",
                info.name,
                handle.version,
                crate::count(p.plan.plan().steps.len(), "step", "steps")
            ),
            Err(e) => {
                failures += 1;
                println!("  FAILED {:<20} {}: {e}", info.name, handle.version);
            }
        }
    }
    if failures == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
