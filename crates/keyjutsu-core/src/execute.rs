//! Executing an approved snapshot through a real terminal session.
//!
//! One plan step at a time: the step's command lines (and its visible
//! validation, unless the plan says to hide it) become a staged performance
//! on the session; when the shell reports them finished, KeyJutsu runs the
//! step's internal checks, records the result, writes a checkpoint and asks
//! the plan's walk what comes next. Branches are decided by KeyJutsu from real
//! outcomes, never by the agent.
//!
//! What this refuses to do:
//!
//! - run a snapshot that was not validated, or whose steps are not all READY;
//! - carry on past a failure: the outcome says what was expected and
//!   what happened;
//! - assume a step that was running when KeyJutsu stopped succeeded:
//!   the checkpoint marks it in doubt and the operator settles it;
//! - reuse a result from an earlier run unless the step's hash is unchanged.

use std::collections::BTreeMap;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use keyjutsu_execution::{
    ExecutionMode as EngineMode, ExecutionState, PerformanceConfig, StagedScript, StagedStep, StepOutcome,
};
use keyjutsu_plan::approval::ApprovedSnapshot;
use keyjutsu_plan::condition::{KnownFacts, StepResult};
use keyjutsu_plan::model::{
    Check, CredentialKind, CredentialRequest, ExecutionMode, Readiness, ServiceState, ShellName, Step,
    StepKind, ValidationDisplay,
};
use keyjutsu_plan::walk::frontier;
use serde::{Deserialize, Serialize};

use crate::session::{Session, SessionEvent, SessionSink};

/// Passes everything to an inner sink and also sends events to a channel,
/// so the execution controller can follow the session without calling back
/// into it from the session's own threads.
pub struct ForwardingSink<S: SessionSink> {
    pub inner: std::sync::Arc<S>,
    pub events: std::sync::Mutex<Sender<SessionEvent>>,
}

impl<S: SessionSink> std::fmt::Debug for ForwardingSink<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForwardingSink").finish_non_exhaustive()
    }
}

impl<S: SessionSink> SessionSink for ForwardingSink<S> {
    fn output(&self, text: &str) {
        self.inner.output(text);
    }
    fn event(&self, event: SessionEvent) {
        if let Ok(tx) = self.events.lock() {
            let _ = tx.send(event.clone());
        }
        self.inner.event(event);
    }
    fn cursor_position(&self) -> (u16, u16) {
        self.inner.cursor_position()
    }
}

/// How one internal check turned out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "execute/")]
pub struct CheckResult {
    pub check: String,
    /// `None` when it could not be decided (for example, a shell with no exit codes).
    pub passed: Option<bool>,
    pub detail: String,
}

/// What happened to one plan step in one run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "execute/")]
pub struct StepRun {
    pub step: String,
    pub step_hash: String,
    pub succeeded: bool,
    pub exit_code: Option<i64>,
    pub checks: Vec<CheckResult>,
    pub started_at: String,
    pub finished_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "execute/")]
pub struct InProgress {
    pub step: String,
    pub step_hash: String,
    pub started_at: String,
}

/// Written before and after every step, so a crash, power loss or restart
/// leaves a record of exactly what is known.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "execute/")]
pub struct Checkpoint {
    pub kind: String,
    pub snapshot_hash: String,
    pub runs: Vec<StepRun>,
    /// A step that had started and not finished. Its effect is unknown.
    pub in_progress: Option<InProgress>,
    /// What each step declared it would change, captured just before it ran.
    /// Latest last.
    #[serde(default)]
    pub captures: Vec<crate::recovery::StepCapture>,
    /// The plan stopped at this boundary and has not yet crossed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub boundary: Option<crate::boundary::BoundaryWait>,
}

const CHECKPOINT_KIND: &str = "keyjutsu.checkpoint/1";

impl Checkpoint {
    pub fn new(snapshot_hash: &str) -> Self {
        Self {
            kind: CHECKPOINT_KIND.into(),
            snapshot_hash: snapshot_hash.into(),
            runs: Vec::new(),
            in_progress: None,
            captures: Vec::new(),
            boundary: None,
        }
    }

    /// Read a checkpoint without asking whether KeyJutsu wrote it. Anything
    /// that acts on one uses `approvals::load_checkpoint` instead.
    pub fn load(path: &Path) -> Result<Self, String> {
        Self::parse(&std::fs::read_to_string(path).map_err(|e| e.to_string())?)
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let c: Checkpoint = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if c.kind != CHECKPOINT_KIND {
            return Err(format!("checkpoint format `{}` is not supported", c.kind));
        }
        Ok(c)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        Self::write(path, &self.to_json())
    }

    /// Write via a temporary file and a rename, so a crash mid-write leaves
    /// the previous checkpoint rather than half of a new one.
    pub(crate) fn write(path: &Path, text: &str) -> Result<(), String> {
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, export_to = "execute/")]
pub enum Outcome {
    Complete,
    /// A step failed; the plan stopped there.
    Failed {
        step: String,
        expected: String,
        actual: String,
        /// The end of what the step printed, without colour codes, so the
        /// agent asked to fix it can read the real error. Redacted
        /// before it is sent anywhere, like everything given to an agent.
        #[serde(default)]
        output: String,
    },
    /// The operator disarmed.
    Aborted {
        step: Option<String>,
        in_doubt: bool,
    },
    /// Execution could not start or continue.
    Blocked {
        reason: String,
    },
    /// Phase `phase` is done and the plan waits for a session boundary.
    /// Resuming checks that it happened.
    Boundary {
        phase: String,
        boundary: keyjutsu_plan::model::Boundary,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, export_to = "execute/")]
pub enum ExecutionEvent {
    StepStarting {
        step: String,
        title: String,
    },
    /// The next step asks the operator for a credential. Staged typing is off
    /// until it is answered; the step starts when the operator presses Enter.
    CredentialRequired {
        step: String,
        prompt: String,
    },
    StepCarried {
        step: String,
    },
    /// An Administrator step ran in the elevation broker's own shell; this
    /// is what it printed (ADR 0011).
    ElevatedOutput {
        step: String,
        text: String,
    },
    Waiting {
        step: String,
        check: String,
    },
    StepFinished {
        step: String,
        run: StepRun,
    },
    Finished {
        outcome: Outcome,
    },
}

/// What the operator is shown before a critical step runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "execute/")]
pub struct CriticalConfirmation {
    pub step: String,
    pub title: String,
    pub commands: Vec<String>,
    /// What it acts on, from the step's declared effects.
    pub targets: Vec<String>,
    /// Why it is needed and what it does, from the plan.
    pub impact: Vec<String>,
    /// How it would be undone, or that it cannot be.
    pub recovery: String,
    /// What validation found.
    pub evidence: Vec<String>,
    /// The phrase to type.
    pub phrase: String,
}

/// How long an approval of a critical step stands on its own. After this, or
/// when the time it was approved cannot be read, the phrase is asked for
/// again before the step runs.
pub const RECONFIRM_AFTER_SECS: u64 = 3_600;

/// Whether a snapshot sealed at `sealed_at` is old enough that its critical
/// steps must be confirmed again before they run.
pub fn needs_reconfirmation(sealed_at: &str, now_secs: u64) -> bool {
    match crate::fingerprint::parse_rfc3339(sealed_at) {
        Some(at) => now_secs.saturating_sub(at) > RECONFIRM_AFTER_SECS,
        None => true,
    }
}

/// Asks the operator to confirm a critical step just before it runs, and
/// returns what they typed, or `None` if they declined. The executor, not
/// the gate, decides whether it matches.
pub type CriticalGate = std::sync::Arc<dyn Fn(&CriticalConfirmation) -> Option<String> + Send + Sync>;

/// How much of a failed step's output is kept: enough for the error and what
/// led to it, not a whole log.
pub const FAILURE_OUTPUT_CHARS: usize = 4000;

fn tail(text: &str, max_chars: usize) -> String {
    let skip = text.chars().count().saturating_sub(max_chars);
    text.chars().skip(skip).collect()
}

#[derive(Clone)]
pub struct ExecuteOptions {
    /// The mode for steps that do not set one; the plan's default otherwise.
    pub mode: Option<EngineMode>,
    pub base: PerformanceConfig,
    pub checkpoint: Option<PathBuf>,
    /// Results the operator has settled for a step left in doubt.
    pub settled: BTreeMap<String, bool>,
    /// How long to wait for the shell to return to its prompt between steps.
    pub prompt_timeout: Duration,
    /// Asked just before a critical step whose approval is more than an
    /// hour old by then. Within the hour, the phrase typed at approval
    /// stands; past it, a run with no gate stops at the step.
    pub critical_gate: Option<CriticalGate>,
    /// Where staged artifacts are kept.
    pub artifact_store: PathBuf,
    /// What identifies the far side of a boundary; the real checks if `None`.
    pub boundary_probe: Option<crate::boundary::BoundaryProbe>,
    /// Asked before resuming after a boundary. Without one, a plan never
    /// resumes past a boundary: never without the operator confirming.
    pub resume_gate: Option<crate::boundary::ResumeGate>,
    /// This machine's environment now; collected for real if `None`.
    pub fingerprint_now: Option<crate::boundary::FingerprintNow>,
    /// Runs Administrator steps when KeyJutsu itself is not elevated.
    pub elevated_runner: Option<std::sync::Arc<dyn crate::elevation::ElevatedRunner>>,
    /// Records each checkpoint written, so a resume can refuse an edited one
    /// (`approvals`). Without it the checkpoint is written unrecorded.
    pub checkpoint_store: Option<std::sync::Arc<crate::store::Store>>,
}

impl Default for ExecuteOptions {
    fn default() -> Self {
        Self {
            mode: None,
            base: PerformanceConfig::default(),
            checkpoint: None,
            settled: BTreeMap::new(),
            prompt_timeout: Duration::from_secs(120),
            critical_gate: None,
            artifact_store: crate::artifacts::default_store(),
            boundary_probe: None,
            resume_gate: None,
            fingerprint_now: None,
            elevated_runner: None,
            checkpoint_store: None,
        }
    }
}

impl std::fmt::Debug for ExecuteOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecuteOptions")
            .field("mode", &self.mode)
            .field("checkpoint", &self.checkpoint)
            .field("settled", &self.settled)
            .field("critical_gate", &self.critical_gate.is_some())
            .field("elevated_runner", &self.elevated_runner.is_some())
            .finish_non_exhaustive()
    }
}

/// The confirmation for a critical step, from the plan and its validation.
pub fn critical_confirmation(plan: &keyjutsu_plan::model::Plan, step: &Step) -> CriticalConfirmation {
    let state = plan.keyjutsu.as_ref().and_then(|k| k.steps.get(&step.id));
    let mut impact: Vec<String> = step.proposed_risk.iter().map(|r| r.rationale.clone()).collect();
    impact.extend(step.reason.iter().cloned());
    if let Some(s) = state {
        impact.extend(s.risk_reasons.iter().cloned());
    }
    let recovery = match (&step.recovery, &step.reversibility) {
        (Some(r), _) if r.strategy == keyjutsu_plan::model::RecoveryStrategy::RestoreCapturedState => {
            "KeyJutsu captures the current state first and can restore it".to_owned()
        }
        (Some(r), _) if r.strategy == keyjutsu_plan::model::RecoveryStrategy::Commands => {
            "the plan has recovery commands".to_owned()
        }
        (_, Some(rev)) if rev.level == keyjutsu_plan::model::ReversibilityLevel::None => format!(
            "NONE: this cannot be undone by KeyJutsu{}",
            rev.notes.as_deref().map(|n| format!(". {n}")).unwrap_or_default()
        ),
        _ => "none declared: KeyJutsu cannot undo this".to_owned(),
    };
    CriticalConfirmation {
        step: step.id.clone(),
        title: step.title.clone(),
        commands: step.commands.iter().map(|c| c.text.clone()).collect(),
        targets: step.expected_effects.iter().map(|e| e.target.clone()).collect(),
        impact,
        recovery,
        evidence: state
            .map(|s| {
                s.evidence
                    .iter()
                    .map(|e| format!("{}: {}", e.check, e.detail.as_deref().unwrap_or("")))
                    .collect()
            })
            .unwrap_or_default(),
        phrase: keyjutsu_plan::approval::confirmation_phrase(step),
    }
}

fn engine_mode(m: ExecutionMode) -> EngineMode {
    match m {
        ExecutionMode::Performance => EngineMode::Performance,
        ExecutionMode::Assisted => EngineMode::Assisted,
        ExecutionMode::AutoPerformance => EngineMode::AutoPerformance,
        ExecutionMode::Direct => EngineMode::Direct,
        ExecutionMode::UserInput => EngineMode::UserInput,
    }
}

/// The staged lines for one plan step: its commands, then its visible
/// validation unless the plan hides it. Operator steps become one user-input
/// line: the operator does what the step says and presses Enter.
/// Put the line that hands a step its artifacts in front of its commands.
/// It is written directly, never performed.
pub fn hand_artifacts(script: &mut StagedScript, step: &Step, line: String) {
    script.steps.insert(
        0,
        StagedStep {
            id: format!("{}#artifacts", step.id),
            title: step.title.clone(),
            command: line,
            mode: Some(EngineMode::Direct),
            submit: None,
            answers: None,
        },
    );
}

pub fn staged_for(step: &Step, show_validation: bool) -> StagedScript {
    if let (StepKind::Credential, Some(request)) = (step.kind, &step.credential) {
        return StagedScript {
            steps: vec![StagedStep {
                id: format!("{}#credential", step.id),
                title: step.title.clone(),
                command: credential_command(request),
                mode: Some(EngineMode::UserInput),
                submit: None,
                answers: Some(credential_answers(request)),
            }],
        };
    }
    let operator = matches!(step.kind, StepKind::Manual | StepKind::UserInput | StepKind::Credential);
    if operator || step.commands.is_empty() {
        return StagedScript {
            steps: vec![StagedStep {
                id: format!("{}#u", step.id),
                title: step.title.clone(),
                command: String::new(),
                mode: Some(EngineMode::UserInput),
                submit: None,
                answers: None,
            }],
        };
    }
    let mode = step.execution_mode.map(engine_mode);
    // A step's working directory is where its commands run. Going there is
    // KeyJutsu's own line, sent directly rather than performed; the approval
    // covers it because the directory is part of the step's hash.
    let enter = step.working_directory.as_ref().map(|dir| {
        let cmd = step.shell.as_ref().is_some_and(|s| s.kind == keyjutsu_plan::model::ShellName::Cmd);
        StagedStep {
            id: format!("{}#cd", step.id),
            title: step.title.clone(),
            command: if cmd {
                format!("cd /d \"{dir}\"")
            } else {
                format!("Set-Location -LiteralPath {}", ps_quote(dir))
            },
            mode: Some(EngineMode::Direct),
            submit: None,
            answers: None,
        }
    });
    let mut lines: Vec<StagedStep> = enter.into_iter().collect();
    lines.extend(step.commands.iter().enumerate().map(|(i, c)| StagedStep {
        id: format!("{}#c{i}", step.id),
        title: step.title.clone(),
        command: c.text.clone(),
        mode,
        submit: None,
        answers: None,
    }));
    if show_validation {
        lines.extend(step.visible_validation.iter().enumerate().map(|(i, c)| StagedStep {
            id: format!("{}#v{i}", step.id),
            title: format!("{} (check)", step.title),
            command: c.text.clone(),
            mode,
            submit: None,
            answers: None,
        }));
    }
    StagedScript { steps: lines }
}

/// A PowerShell single-quoted string. PowerShell also ends such a string at
/// the typographic single quotes, so those are doubled as well.
pub(crate) fn ps_quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('\'');
    for c in text.chars() {
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            out.push(c);
        }
        out.push(c);
    }
    out.push('\'');
    out
}

/// The command that asks for a credential. The shell's own prompt
/// masks what the operator types, keeps it out of history, and holds it as a
/// SecureString or PSCredential; KeyJutsu passes the keys through and never
/// holds the secret. The variable name is checked by the schema.
pub fn credential_command(request: &CredentialRequest) -> String {
    let prompt = ps_quote(&request.prompt);
    match request.kind {
        CredentialKind::Secret => {
            format!("${} = Read-Host -AsSecureString -Prompt {prompt}", request.variable)
        }
        CredentialKind::UsernameAndPassword => {
            let user = request.username.as_deref().map(|u| format!(" -UserName {}", ps_quote(u)));
            format!("${} = Get-Credential -Message {prompt}{}", request.variable, user.unwrap_or_default())
        }
    }
}

/// How many Enters answer the prompt: one for a secret, or for a password
/// when the user name is given; two when the user name is asked for too.
pub fn credential_answers(request: &CredentialRequest) -> u32 {
    match (request.kind, &request.username) {
        (CredentialKind::UsernameAndPassword, None) => 2,
        _ => 1,
    }
}

/// Removes the credentials a run asked for from the shell: they last only as long as the run.
pub fn forget_command(variables: &[String]) -> String {
    format!("Remove-Variable -Name {} -Scope Global -ErrorAction Ignore", variables.join(","))
}

/// Run one internal check now.
pub fn run_check(check: &Check, last_exit: Option<i64>) -> CheckResult {
    let (name, passed, detail): (String, Option<bool>, String) = match check {
        Check::ExitCode { equals } => match last_exit {
            Some(code) => {
                ("exit code".into(), Some(code == *equals), format!("expected {equals}, got {code}"))
            }
            None => ("exit code".into(), None, "this shell reports no exit codes".into()),
        },
        Check::ServiceState(s) => {
            let actual = service_state(&s.name);
            let want = format!("{:?}", s.state).to_ascii_lowercase();
            (
                format!("service {}", s.name),
                actual.as_ref().map(|a| *a == want),
                format!("expected {want}, got {}", actual.as_deref().unwrap_or("unknown")),
            )
        }
        Check::PathExists(p) => {
            let exists = Path::new(&p.path).exists();
            (
                format!("path {}", p.path),
                Some(exists),
                if exists { "exists".into() } else { "does not exist".into() },
            )
        }
        Check::FileSha256 { path, sha256 } => match std::fs::read(path) {
            Ok(bytes) => {
                let actual = keyjutsu_plan::hash::sha256_hex(&bytes);
                (
                    format!("sha256 of {path}"),
                    Some(actual.eq_ignore_ascii_case(sha256)),
                    format!("got {actual}"),
                )
            }
            Err(e) => (format!("sha256 of {path}"), Some(false), e.to_string()),
        },
        Check::JsonValue { path, pointer, equals } => {
            let actual = std::fs::read_to_string(path)
                .ok()
                .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                .and_then(|v| v.pointer(pointer).cloned());
            (
                format!("{path}{pointer}"),
                Some(actual.as_ref() == Some(equals)),
                format!(
                    "expected {equals}, got {}",
                    actual.map(|a| a.to_string()).unwrap_or_else(|| "nothing".into())
                ),
            )
        }
        Check::TcpPortOpen { host, port } => {
            let open = (host.as_str(), *port)
                .to_socket_addrs()
                .ok()
                .into_iter()
                .flatten()
                .any(|a| TcpStream::connect_timeout(&a, Duration::from_secs(3)).is_ok());
            (
                format!("port {host}:{port}"),
                Some(open),
                if open { "open".into() } else { "not accepting connections".into() },
            )
        }
        Check::HttpStatus { url, status } => {
            let actual = http_status(url);
            (
                format!("HTTP {url}"),
                actual.map(|a| a == *status),
                format!(
                    "expected {status}, got {}",
                    actual.map(|a| a.to_string()).unwrap_or_else(|| "no response".into())
                ),
            )
        }
        Check::TimeoutSeconds(s) => ("wait".into(), Some(true), format!("up to {s} seconds")),
    };
    CheckResult { check: name, passed, detail }
}

fn service_state(name: &str) -> Option<String> {
    let ps = keyjutsu_terminal::shell::locate(keyjutsu_terminal::ShellKind::Pwsh)
        .or_else(|| keyjutsu_terminal::shell::locate(keyjutsu_terminal::ShellKind::WindowsPowershell))?;
    let a = keyjutsu_validation::powershell::analyse(&ps, &[], &[], &[name]).ok()?;
    a.services.get(name).cloned()
}

fn http_status(url: &str) -> Option<u16> {
    let ps = keyjutsu_terminal::shell::locate(keyjutsu_terminal::ShellKind::Pwsh)?;
    let quoted = url.replace('\'', "''");
    let script = format!(
        "try {{ (Invoke-WebRequest -Uri '{quoted}' -Method Head -UseBasicParsing -TimeoutSec 10 -SkipHttpErrorCheck).StatusCode }} catch {{ -1 }}"
    );
    let mut c = std::process::Command::new(ps);
    c.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-EncodedCommand"]);
    c.arg(keyjutsu_terminal::shell::encode_powershell_command(&script));
    let done = keyjutsu_validation::process::run(c, "", Duration::from_secs(20)).ok()?;
    done.stdout.trim().parse::<i32>().ok().and_then(|n| u16::try_from(n).ok())
}

/// Run a step's internal checks, waiting up to its `timeout_seconds` for
/// them to hold.
fn run_checks(step: &Step, last_exit: Option<i64>, observe: &dyn Fn(ExecutionEvent)) -> Vec<CheckResult> {
    let wait = step.internal_validation.iter().find_map(|c| match c {
        Check::TimeoutSeconds(s) => Some(Duration::from_secs(u64::from(*s))),
        _ => None,
    });
    let checks: Vec<&Check> =
        step.internal_validation.iter().filter(|c| !matches!(c, Check::TimeoutSeconds(_))).collect();
    let deadline = Instant::now() + wait.unwrap_or_default();
    loop {
        let results: Vec<CheckResult> = checks.iter().map(|c| run_check(c, last_exit)).collect();
        let failing = results.iter().find(|r| r.passed == Some(false));
        match failing {
            Some(f) if Instant::now() < deadline => {
                observe(ExecutionEvent::Waiting { step: step.id.clone(), check: f.check.clone() });
                std::thread::sleep(Duration::from_secs(1));
            }
            _ => return results,
        }
    }
}

/// Everything `execute` needs to know about the session it drives.
pub struct Driver<'a> {
    pub session: &'a Session,
    pub events: &'a Receiver<SessionEvent>,
}

impl std::fmt::Debug for Driver<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Driver").finish_non_exhaustive()
    }
}

/// How a staged performance ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Performed {
    /// Every line reported a result (some may have failed).
    Finished(Vec<StepOutcome>),
    /// Disarmed, or the shell went away. `in_doubt` when a command had been
    /// submitted, so its effect is unknown.
    Unfinished { in_doubt: bool },
    /// The session would not arm it.
    Refused(String),
}

/// Arm `script` on the session and follow it until it ends.
pub fn perform(driver: &Driver<'_>, script: StagedScript, config: PerformanceConfig) -> Performed {
    let lines = script.steps.len();
    // Drop events left over from before.
    while driver.events.try_recv().is_ok() {}
    if let Err(e) = driver.session.arm(script, config) {
        return Performed::Refused(e.to_string());
    }
    let mut outcomes: Vec<StepOutcome> = Vec::new();
    let mut last_state = ExecutionState::Armed;
    // The last state before the performance ended, to tell whether a
    // command had been submitted when it stopped.
    let mut live_state = ExecutionState::Armed;
    let ended = loop {
        match driver.events.recv_timeout(Duration::from_millis(500)) {
            Ok(SessionEvent::StepFinished { outcome, .. }) => outcomes.push(outcome),
            // The session sends each line's result before the snapshot that
            // reports the state it led to, so by the time the state is final
            // every result has arrived.
            Ok(SessionEvent::Performance { snapshot: s }) => {
                last_state = s.state;
                if !s.state.is_terminal() && s.state != ExecutionState::Failed {
                    live_state = s.state;
                }
                if matches!(s.state, ExecutionState::Complete | ExecutionState::Failed) {
                    break Some(s.state);
                }
            }
            Ok(SessionEvent::Released)
                if !matches!(last_state, ExecutionState::Complete | ExecutionState::Failed) =>
            {
                break None;
            }
            Ok(SessionEvent::Exited { .. }) => break None,
            Ok(_) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break None,
        }
    };
    // A performance that ended without a result for every line did not
    // finish: the shell exited under it. That is never a success.
    let unfinished = match ended {
        None => true,
        Some(ExecutionState::Complete) => outcomes.len() != lines,
        Some(_) => !outcomes.iter().any(|o| matches!(o, StepOutcome::Failed { .. })),
    };
    if unfinished {
        let in_doubt = matches!(
            live_state,
            ExecutionState::Executing | ExecutionState::Waiting | ExecutionState::Validating
        );
        return Performed::Unfinished { in_doubt };
    }
    Performed::Finished(outcomes)
}

/// Why `snapshot` may not run on this machine as it is `now`, if a step
/// not yet run in `done` depends on something that changed since approval.
/// Front ends ask before starting anything (a broker's UAC prompt, say);
/// the executor asks again, so no front end can skip it.
pub fn changed_since_approval(
    snapshot: &ApprovedSnapshot,
    now: &keyjutsu_plan::hash::EnvironmentFingerprint,
    done: &Checkpoint,
) -> Option<String> {
    let drifts = snapshot.fingerprint()?.drift(now);
    let affected = still_to_run(
        keyjutsu_plan::hash::affected_by_drift(snapshot.plan(), snapshot.graph(), &drifts),
        done,
    );
    (!affected.is_empty()).then(|| {
        format!(
            "this machine has changed since the plan was approved ({}); steps {} require revalidation",
            describe_drift(&drifts),
            affected.join(", ")
        )
    })
}

/// The steps in `affected` that have not already run.
fn still_to_run(affected: Vec<String>, checkpoint: &Checkpoint) -> Vec<String> {
    affected.into_iter().filter(|s| !checkpoint.runs.iter().any(|r| &r.step == s)).collect()
}

/// What changed, as `what before -> after; …`.
fn describe_drift(drifts: &[keyjutsu_plan::hash::Drift]) -> String {
    drifts
        .iter()
        .map(|d| {
            format!(
                "{} {} -> {}",
                d.what,
                d.before.as_deref().unwrap_or("absent"),
                d.after.as_deref().unwrap_or("absent")
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Why a snapshot may not be executed at all.
pub fn preflight(snapshot: &ApprovedSnapshot) -> Result<(), String> {
    let Some(state) = &snapshot.plan().keyjutsu else {
        return Err("the snapshot was not validated before it was sealed; no unvalidated step may run".into());
    };
    let not_ready: Vec<&str> = snapshot
        .graph()
        .topological_order()
        .filter(|id| state.steps.get(*id).map(|s| s.readiness) != Some(Readiness::Ready))
        .collect();
    if !not_ready.is_empty() {
        return Err(format!("steps not ready: {}", not_ready.join(", ")));
    }
    let mut shells = Vec::new();
    for kind in snapshot.plan().steps.iter().filter_map(|s| s.shell.as_ref().map(|sh| sh.kind)) {
        if !shells.contains(&kind) {
            shells.push(kind);
        }
    }
    if shells.len() > 1 {
        return Err(
            "the plan uses more than one shell; a performance runs in one terminal (deviation D19)".into()
        );
    }
    for step in snapshot.plan().steps.iter().filter(|s| s.kind == StepKind::Credential) {
        if step.credential.is_none() {
            return Err(format!("credential step `{}` does not say what it asks for", step.id));
        }
        if shells.contains(&ShellName::Cmd) {
            return Err(format!(
                "credential step `{}` needs PowerShell: cmd has no masked prompt, so the secret would be shown",
                step.id
            ));
        }
    }
    Ok(())
}

/// Execute `snapshot` on the session. Blocks until the plan completes, fails,
/// is disarmed or cannot continue; `observe` hears every step as it goes.
pub fn execute(
    driver: &Driver<'_>,
    snapshot: &ApprovedSnapshot,
    resume: Option<Checkpoint>,
    options: &ExecuteOptions,
    now: &dyn Fn() -> String,
    observe: &dyn Fn(ExecutionEvent),
) -> (Outcome, Checkpoint) {
    let mut checkpoint = Checkpoint::new(snapshot.snapshot_hash());
    let finish = |outcome: Outcome, checkpoint: Checkpoint| {
        // Anything short of completion hands the keyboard back. A step
        // that failed only its checks left a performance that had completed,
        // and a completed performance keeps the keyboard until disarmed.
        if outcome != Outcome::Complete {
            driver.session.disarm();
        }
        observe(ExecutionEvent::Finished { outcome: outcome.clone() });
        (outcome, checkpoint)
    };
    if let Err(reason) = preflight(snapshot) {
        return finish(Outcome::Blocked { reason }, checkpoint);
    }
    // Everything the plan downloads must be staged and verified before it
    // arms: nothing is fetched while it runs.
    for a in crate::artifacts::artifacts(snapshot.plan()) {
        if let Err(reason) = crate::artifacts::verify(&options.artifact_store, a) {
            return finish(Outcome::Blocked { reason }, checkpoint);
        }
    }
    let plan = snapshot.plan();
    let hashes = snapshot.step_hashes();
    let show_validation =
        plan.execution_preferences.as_ref().and_then(|p| p.show_validation) != Some(ValidationDisplay::None);
    let default_mode = options
        .mode
        .or_else(|| plan.execution_preferences.as_ref().and_then(|p| p.default_mode).map(engine_mode))
        .unwrap_or(options.base.mode);
    let try_save = |c: &Checkpoint| -> Result<(), String> {
        match (&options.checkpoint, &options.checkpoint_store) {
            (Some(path), Some(store)) => crate::approvals::save_checkpoint(store, c, path),
            (Some(path), None) => c.save(path),
            (None, _) => Ok(()),
        }
    };
    // Most saves are a best effort: the run is better finished than stopped
    // for a record. The one before a step starts is not (below).
    let save = |c: &Checkpoint| {
        let _ = try_save(c);
    };

    // Results from an earlier run count only where the step is unchanged.
    let mut facts = KnownFacts::default();
    let pending = resume.as_ref().and_then(|r| r.boundary.clone());
    if let Some(previous) = resume {
        if let Some(doubt) = &previous.in_progress {
            match options.settled.get(&doubt.step) {
                None => {
                    return finish(
                        Outcome::Blocked {
                            reason: format!(
                                "step `{}` was running when KeyJutsu last stopped, so its effect is unknown; check the machine and settle it",
                                doubt.step
                            ),
                        },
                        previous,
                    );
                }
                Some(&ok) if hashes.get(&doubt.step) == Some(&doubt.step_hash) && ok => {
                    checkpoint.runs.push(StepRun {
                        step: doubt.step.clone(),
                        step_hash: doubt.step_hash.clone(),
                        succeeded: true,
                        exit_code: None,
                        checks: vec![CheckResult {
                            check: "settled by the operator".into(),
                            passed: Some(true),
                            detail: String::new(),
                        }],
                        started_at: doubt.started_at.clone(),
                        finished_at: now(),
                    });
                }
                Some(_) => {}
            }
        }
        // A credential lives only in the shell that asked for it, so a
        // resumed run asks again.
        let asks = |id: &str| plan.step(id).is_some_and(|s| s.kind == StepKind::Credential);
        for run in previous.runs.into_iter().filter(|r| r.succeeded && !asks(&r.step)) {
            if hashes.get(&run.step) == Some(&run.step_hash) {
                checkpoint.runs.push(run);
            }
        }
        checkpoint.captures =
            previous.captures.into_iter().filter(|c| hashes.get(&c.step) == Some(&c.step_hash)).collect();
        for run in &checkpoint.runs {
            facts.steps.insert(run.step.clone(), StepResult::Succeeded { exit_code: run.exit_code });
            observe(ExecutionEvent::StepCarried { step: run.step.clone() });
        }
    }
    // The machine is compared with the one the plan was approved on before
    // anything runs, whichever front end started the run. Across a
    // boundary the comparison comes below, once the boundary is confirmed.
    if pending.is_none() && snapshot.fingerprint().is_some() {
        let now_fp = match &options.fingerprint_now {
            Some(f) => f(plan),
            None => crate::fingerprint::collect(Some(plan)),
        };
        if let Some(reason) = changed_since_approval(snapshot, &now_fp, &checkpoint) {
            return finish(Outcome::Blocked { reason }, checkpoint);
        }
    }
    save(&checkpoint);

    // Boundaries. One is behind the plan once any step after it ran.
    let probe = options.boundary_probe.clone().unwrap_or_else(crate::boundary::real_probe);
    let phase_of = |id: &str| plan.phases.iter().position(|p| p.steps.iter().any(|s| s == id));
    let mut crossed: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
    for run in &checkpoint.runs {
        if let Some(p) = phase_of(&run.step) {
            crossed.extend(0..p);
        }
    }
    if let Some(wait) = pending {
        checkpoint.boundary = Some(wait.clone());
        let what = crate::boundary::describe(wait.kind);
        let block = |reason: String, checkpoint: Checkpoint| {
            save(&checkpoint);
            finish(Outcome::Blocked { reason }, checkpoint)
        };
        let Some(k) = plan.phases.iter().position(|p| p.id == wait.after_phase) else {
            return block(
                format!(
                    "the checkpoint waits after phase `{}`, which this plan does not have",
                    wait.after_phase
                ),
                checkpoint,
            );
        };
        // It must really have happened.
        let now_id = probe(wait.kind, driver.session);
        let verified = match (&wait.identity, &now_id) {
            (Some(before), Some(now)) if before == now => {
                return block(format!("the {what} has not happened yet; resume after it"), checkpoint);
            }
            (Some(_), Some(_)) => true,
            _ => false,
        };
        // The machine is compared with the approved one again.
        let now_fp = match &options.fingerprint_now {
            Some(f) => f(plan),
            None => crate::fingerprint::collect(Some(plan)),
        };
        let drifts = snapshot.fingerprint().map(|then| then.drift(&now_fp)).unwrap_or_default();
        let affected = still_to_run(
            keyjutsu_plan::hash::affected_by_drift(plan, snapshot.graph(), &drifts),
            &checkpoint,
        );
        if !affected.is_empty() {
            return block(
                format!(
                    "after the {what}, this machine differs from the one the plan was approved on ({}); steps {} require revalidation",
                    describe_drift(&drifts),
                    affected.join(", ")
                ),
                checkpoint,
            );
        }
        // What the earlier phases achieved is checked again, not assumed.
        let mut rechecked = Vec::new();
        for run in &checkpoint.runs {
            let Some(step) = plan.step(&run.step) else { continue };
            for c in step
                .internal_validation
                .iter()
                .filter(|c| !matches!(c, Check::ExitCode { .. } | Check::TimeoutSeconds(_)))
            {
                let mut r = run_check(c, None);
                r.check = format!("{}: {}", run.step, r.check);
                rechecked.push(r);
            }
        }
        if let Some(bad) = rechecked.iter().find(|r| r.passed == Some(false)) {
            return block(
                format!(
                    "after the {what}, what an earlier step achieved no longer holds: {}: {}",
                    bad.check, bad.detail
                ),
                checkpoint,
            );
        }
        let notice = crate::boundary::BoundaryNotice {
            kind: wait.kind,
            after_phase: wait.after_phase.clone(),
            next_phase: plan.phases.get(k + 1).map(|p| p.id.clone()),
            verified,
            rechecked,
            drifts,
        };
        match &options.resume_gate {
            None => {
                return block(
                    format!("resuming after a {what} needs the operator's confirmation"),
                    checkpoint,
                );
            }
            Some(gate) if !gate(&notice) => {
                return block(format!("the operator did not confirm resuming after the {what}"), checkpoint);
            }
            Some(_) => {}
        }
        crossed.insert(k);
        checkpoint.boundary = None;
        save(&checkpoint);
    }

    // Credentials asked for in this run, removed from the shell at the end.
    let mut asked: Vec<String> = Vec::new();
    let forget = |asked: &[String]| {
        if !asked.is_empty() && driver.session.wait_for_prompt(options.prompt_timeout) {
            let script = StagedScript {
                steps: vec![StagedStep {
                    id: "forget-credentials".into(),
                    title: "Forget credentials".into(),
                    command: forget_command(asked),
                    mode: Some(EngineMode::Direct),
                    submit: None,
                    answers: None,
                }],
            };
            while driver.events.try_recv().is_ok() {}
            if driver
                .session
                .arm(script, PerformanceConfig { mode: EngineMode::Direct, ..options.base.clone() })
                .is_ok()
            {
                let _ = driver.session.wait_for_prompt(options.prompt_timeout);
            }
        }
    };

    loop {
        let f = frontier(plan, snapshot.graph(), &facts);
        if f.complete {
            save(&checkpoint);
            forget(&asked);
            return finish(Outcome::Complete, checkpoint);
        }
        let Some(id) = f.next().map(str::to_owned) else {
            let needs: Vec<String> = f.needs.iter().map(|m| format!("{m:?}")).collect();
            return finish(
                Outcome::Blocked {
                    reason: format!("no step can run until these are known: {}", needs.join(", ")),
                },
                checkpoint,
            );
        };
        for skipped in &f.skipped {
            facts.steps.entry(skipped.clone()).or_insert(StepResult::Skipped);
        }
        let Some(step) = plan.step(&id) else {
            return finish(Outcome::Blocked { reason: format!("step `{id}` vanished") }, checkpoint);
        };
        let step_hash = hashes.get(&id).cloned().unwrap_or_default();

        // A step after an uncrossed boundary waits for it: stop here, and
        // record what should be different on the other side.
        if let Some(p) = phase_of(&id) {
            let waiting = (0..p).find(|k| plan.phases[*k].boundary_after.is_some() && !crossed.contains(k));
            if let Some(k) = waiting
                && let Some(kind) = plan.phases[k].boundary_after
            {
                checkpoint.boundary = Some(crate::boundary::BoundaryWait {
                    after_phase: plan.phases[k].id.clone(),
                    kind,
                    identity: probe(kind, driver.session),
                    recorded_at: now(),
                });
                save(&checkpoint);
                forget(&asked);
                return finish(
                    Outcome::Boundary { phase: plan.phases[k].id.clone(), boundary: kind },
                    checkpoint,
                );
            }
        }

        if !driver.session.wait_for_prompt(options.prompt_timeout) {
            return finish(
                Outcome::Blocked { reason: "the shell did not return to its prompt".into() },
                checkpoint,
            );
        }
        // A critical step approved more than an hour ago is confirmed again,
        // just before it runs: judged now, not when the run began, so a long
        // run cannot carry an old approval past its hour.
        let now_secs = crate::fingerprint::parse_rfc3339(&now()).unwrap_or(u64::MAX);
        if keyjutsu_plan::approval::is_critical(plan, step)
            && needs_reconfirmation(snapshot.sealed_at(), now_secs)
        {
            let Some(gate) = &options.critical_gate else {
                save(&checkpoint);
                return finish(
                    Outcome::Blocked {
                        reason: format!(
                            "critical step `{id}` was approved more than an hour ago and must be confirmed again before it runs, and nothing here can ask"
                        ),
                    },
                    checkpoint,
                );
            };
            let ask = critical_confirmation(plan, step);
            let typed = gate(&ask);
            if typed.as_deref().map(str::trim) != Some(ask.phrase.as_str()) {
                save(&checkpoint);
                return finish(
                    Outcome::Blocked {
                        reason: format!(
                            "critical step `{id}` was not confirmed, so it did not run{}",
                            if typed.is_some() { " (the phrase did not match)" } else { "" }
                        ),
                    },
                    checkpoint,
                );
            }
        }
        // An Administrator step, with KeyJutsu itself unelevated, goes to the
        // elevation broker, which checks it against its own copy of the
        // approved snapshot. It is never typed into the unelevated shell,
        // and the broker captures what it declares itself (ADR 0017).
        let needs_broker = step.privilege == Some(keyjutsu_plan::model::Privilege::Administrator)
            && !crate::elevation::is_elevated();
        // Prepare the step's recovery before it runs, or do not run it.
        if !needs_broker && step.recovery.as_ref().is_some_and(|r| !r.capture.is_empty()) {
            let Some(dir) = options.checkpoint.as_deref().map(crate::recovery::recovery_dir) else {
                return finish(
                    Outcome::Blocked {
                        reason: format!(
                            "step `{id}` captures state for recovery, but this run keeps no checkpoint"
                        ),
                    },
                    checkpoint,
                );
            };
            match crate::recovery::capture_step(step, &step_hash, &dir, now()) {
                Ok(c) => checkpoint.captures.push(c),
                Err(e) => {
                    save(&checkpoint);
                    return finish(
                        Outcome::Blocked { reason: format!("could not prepare recovery for `{id}`: {e}") },
                        checkpoint,
                    );
                }
            }
        }
        let started_at = now();
        // Where this step's output begins, for the failure report.
        let output_from = driver.session.output_mark();
        let mut elevated_output: Option<String> = None;
        checkpoint.in_progress = Some(InProgress {
            step: id.clone(),
            step_hash: step_hash.clone(),
            started_at: started_at.clone(),
        });
        // A step does not start unless the record that it started is safely
        // written: after a crash, that record is what says its effect is in
        // doubt rather than that it never ran.
        if let Err(e) = try_save(&checkpoint) {
            checkpoint.in_progress = None;
            return finish(
                Outcome::Blocked {
                    reason: format!("could not record that step `{id}` is starting, so it has not run: {e}"),
                },
                checkpoint,
            );
        }
        observe(ExecutionEvent::StepStarting { step: id.clone(), title: step.title.clone() });
        if let (StepKind::Credential, Some(request)) = (step.kind, &step.credential) {
            observe(ExecutionEvent::CredentialRequired { step: id.clone(), prompt: request.prompt.clone() });
            if !asked.contains(&request.variable) {
                asked.push(request.variable.clone());
            }
        }

        let mut script = staged_for(step, show_validation);
        // Hand the step its staged artifacts, checked again just before it
        // runs; a copy that changed since staging stops the plan here.
        match crate::artifacts::assignment(&options.artifact_store, step) {
            Ok(Some(line)) => hand_artifacts(&mut script, step, line),
            Ok(None) => {}
            Err(reason) => {
                checkpoint.in_progress = None;
                save(&checkpoint);
                return finish(Outcome::Blocked { reason }, checkpoint);
            }
        }
        let config = PerformanceConfig { mode: default_mode, ..options.base.clone() };
        let performed = if needs_broker {
            let Some(runner) = &options.elevated_runner else {
                checkpoint.in_progress = None;
                save(&checkpoint);
                return finish(
                    Outcome::Blocked {
                        reason: format!(
                            "step `{id}` needs Administrator, and no elevation broker is running for this plan"
                        ),
                    },
                    checkpoint,
                );
            };
            match runner.run_step(snapshot.snapshot_hash(), &id, &step_hash) {
                Ok(run) => {
                    // The broker's account of what it captured, for the
                    // recovery plan to show.
                    if let Some(c) = run.captured {
                        checkpoint.captures.push(c);
                    }
                    elevated_output = Some(run.output.clone());
                    observe(ExecutionEvent::ElevatedOutput { step: id.clone(), text: run.output });
                    Performed::Finished(run.outcomes)
                }
                // The broker may have started the step before failing: its
                // effect is unknown, so the step stays in doubt.
                Err(reason) => {
                    save(&checkpoint);
                    return finish(
                        Outcome::Blocked { reason: format!("the elevation broker: {reason}") },
                        checkpoint,
                    );
                }
            }
        } else {
            perform(driver, script, config)
        };
        let outcomes = match performed {
            Performed::Finished(outcomes) => outcomes,
            Performed::Refused(reason) => {
                checkpoint.in_progress = None;
                save(&checkpoint);
                return finish(Outcome::Blocked { reason }, checkpoint);
            }
            Performed::Unfinished { in_doubt } => {
                if !in_doubt {
                    checkpoint.in_progress = None;
                }
                save(&checkpoint);
                return finish(Outcome::Aborted { step: Some(id), in_doubt }, checkpoint);
            }
        };

        let last_exit = outcomes.iter().rev().find_map(|o| match o {
            StepOutcome::Succeeded { exit_code } | StepOutcome::Failed { exit_code } => {
                Some(i64::from(*exit_code))
            }
            StepOutcome::Unverified => None,
        });
        let failed_line = outcomes.iter().find_map(|o| match o {
            StepOutcome::Failed { exit_code } => Some(*exit_code),
            _ => None,
        });
        let checks = if failed_line.is_none() { run_checks(step, last_exit, observe) } else { Vec::new() };
        let failed_check = checks.iter().find(|c| c.passed == Some(false));
        let succeeded = failed_line.is_none() && failed_check.is_none();
        let run = StepRun {
            step: id.clone(),
            step_hash,
            succeeded,
            exit_code: last_exit,
            checks: checks.clone(),
            started_at,
            finished_at: now(),
        };
        checkpoint.in_progress = None;
        checkpoint.runs.push(run.clone());
        save(&checkpoint);
        observe(ExecutionEvent::StepFinished { step: id.clone(), run });

        if !succeeded {
            let (expected, actual) = match (failed_line, failed_check) {
                (Some(code), _) => {
                    ("every command to succeed".to_owned(), format!("a command exited with {code}"))
                }
                (None, Some(c)) => (c.check.clone(), c.detail.clone()),
                (None, None) => (String::new(), String::new()),
            };
            forget(&asked);
            let output = match elevated_output {
                Some(text) => tail(&crate::headless::strip_ansi(&text), FAILURE_OUTPUT_CHARS),
                None => driver.session.output_since(output_from, FAILURE_OUTPUT_CHARS),
            };
            return finish(Outcome::Failed { step: id, expected, actual, output }, checkpoint);
        }
        facts.steps.insert(id, StepResult::Succeeded { exit_code: last_exit });
    }
}

/// `ServiceState` as a condition value, for callers that report checks.
pub fn service_state_name(s: ServiceState) -> &'static str {
    match s {
        ServiceState::Running => "running",
        ServiceState::Stopped => "stopped",
        ServiceState::Paused => "paused",
        ServiceState::Missing => "missing",
    }
}
