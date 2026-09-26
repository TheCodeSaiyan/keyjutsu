//! The KeyJutsu desktop app's Rust side.
//!
//! The React front end can only ask. Every command here hands the request to
//! keyjutsu-core, which decides whether it is allowed: raw input while a
//! performance owns the keyboard is refused there, arming is checked there,
//! the demo script is built there rather than accepted from the window, and
//! every plan edit is re-read as a whole plan there before it is kept.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Condvar, Mutex};

use keyjutsu_core::agent::context::{ContextItem, prepare};
use keyjutsu_core::agent::{AgentHandle, AgentInfo, AgentKind, Agents, ProcessRunner, detect_all};
use keyjutsu_core::execute::{
    Checkpoint, CriticalConfirmation, CriticalGate, Driver, ExecuteOptions, execute,
};
use keyjutsu_core::execution::{
    ExecutionState, PerformanceConfig, PerformanceSnapshot, StagedScript, StagedStep,
};
use keyjutsu_core::ipc::{
    OpenRequest, RunMessage, ScriptSource, Sealed, TerminalMessage, WaitingOpened, WaitingRun,
};
use keyjutsu_core::plan::ApprovedSnapshot;
use keyjutsu_core::plan::model::Step;
use keyjutsu_core::readiness::{self, ReadinessReport};
use keyjutsu_core::recovery::{RecoveryItem, RecoveryResult, plan_recovery, recover, recovery_dir};
use keyjutsu_core::session::is_terminal_report;
use keyjutsu_core::terminal::profile::{self, TerminalProfile};
use keyjutsu_core::terminal::{KeyChord, TerminalSize};
use keyjutsu_core::validation::Options;
use keyjutsu_core::workspace::{Workspace, WorkspaceView, run_folder};
use keyjutsu_core::{CoreError, Session, SessionEvent, SessionOptions, SessionSink, demo};
use keyjutsu_core::{fingerprint, git};
use tauri::State;
use tauri::ipc::Channel;

/// Sends a session's output and events to its terminal view, and its events
/// to a running plan's executor as well, while one runs.
struct ChannelSink {
    channel: Channel<TerminalMessage>,
    forward: Mutex<Option<Sender<SessionEvent>>>,
}

impl SessionSink for ChannelSink {
    fn output(&self, text: &str) {
        let _ = self.channel.send(TerminalMessage::Output { data: text.to_owned() });
    }

    fn event(&self, event: SessionEvent) {
        if let Ok(forward) = self.forward.lock()
            && let Some(tx) = forward.as_ref()
        {
            let _ = tx.send(event.clone());
        }
        let _ = self.channel.send(TerminalMessage::Event { event });
    }
}

#[derive(Default)]
struct Sessions {
    next: AtomicU32,
    open: Mutex<HashMap<u32, (Session, Arc<ChannelSink>)>>,
}

impl Sessions {
    fn get(&self, id: u32) -> Result<(Session, Arc<ChannelSink>), String> {
        self.open
            .lock()
            .map_err(|e| e.to_string())?
            .get(&id)
            .cloned()
            .ok_or_else(|| format!("no terminal session {id}"))
    }
}

/// The plan being worked on, the snapshot sealed from it, and the run.
#[derive(Default)]
struct Plans {
    workspace: Mutex<Option<Workspace>>,
    sealed: Mutex<Option<(ApprovedSnapshot, PathBuf)>>,
    /// The latest run's checkpoint, for recovery.
    checkpoint: Mutex<Option<PathBuf>>,
    /// The latest run's failure, if it failed: which step, how, and that
    /// run's checkpoint. An agent fixing the step reads it, and the next run
    /// resumes from the checkpoint instead of starting again.
    failed: Mutex<Option<Failed>>,
    /// The checkpoint of a run stopped at a session boundary, waiting to
    /// cross it. The next run of the same snapshot resumes from it.
    waiting: Mutex<Option<PathBuf>>,
    /// The operator's answer to a critical confirmation: `Some(None)` declines.
    answer: Arc<(Mutex<Option<Option<String>>>, Condvar)>,
}

#[derive(Clone)]
struct Failed {
    step: String,
    failure: keyjutsu_core::agent::RunFailure,
    checkpoint: PathBuf,
}

fn message(e: CoreError) -> String {
    match e {
        // A stable token the front end can recognise and ignore quietly.
        CoreError::InputOwned => "input_owned".into(),
        other => other.to_string(),
    }
}

fn locked<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[tauri::command]
async fn readiness_scan() -> Result<ReadinessReport, String> {
    tauri::async_runtime::spawn_blocking(readiness::scan).await.map_err(|e| e.to_string())
}

/// The bundle last shown to the operator: only that is ever saved, so what
/// they read is what they get.
#[derive(Default)]
struct Diagnostics(Mutex<Option<String>>);

#[tauri::command]
async fn diagnostics_preview(diagnostics: State<'_, Arc<Diagnostics>>) -> Result<String, String> {
    let text = tauri::async_runtime::spawn_blocking(keyjutsu_core::diagnostics::collect)
        .await
        .map_err(|e| e.to_string())?;
    *locked(&diagnostics.0) = Some(text.clone());
    Ok(text)
}

/// Saves the previewed bundle and shows it in Explorer. Nothing is sent.
#[tauri::command]
fn diagnostics_save(diagnostics: State<'_, Arc<Diagnostics>>) -> Result<String, String> {
    let text = locked(&diagnostics.0).clone();
    let text = text.ok_or("preview the bundle first: only a bundle you have seen is saved")?;
    let file = keyjutsu_core::diagnostics::save(&text, &keyjutsu_core::diagnostics::default_dir())?;
    let mut select = std::ffi::OsString::from("/select,");
    select.push(&file);
    let _ = std::process::Command::new("explorer.exe").arg(select).spawn();
    Ok(file.display().to_string())
}

#[tauri::command]
fn terminal_profile() -> TerminalProfile {
    profile::detect_terminal_profile()
}

/// The folder "Open KeyJutsu here" was used on, if it was.
static LAUNCH_FOLDER: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// A terminal as the window asked for it, started in the launch folder if
/// there is one. Without it a shell starts in the user's home folder, and
/// "Open KeyJutsu here" would open everywhere but there.
fn session_options(request: &OpenRequest, launch_folder: Option<PathBuf>) -> SessionOptions {
    let mut options = SessionOptions::new(request.shell);
    options.profile = request.profile;
    options.size = request.size;
    options.cwd = launch_folder;
    // xterm.js answers ConPTY's cursor queries itself.
    options.intercept_cursor_queries = false;
    options
}

#[tauri::command]
fn terminal_open(
    request: OpenRequest,
    on_message: Channel<TerminalMessage>,
    sessions: State<'_, Arc<Sessions>>,
) -> Result<u32, String> {
    let options = session_options(&request, LAUNCH_FOLDER.get().cloned());
    let sink = Arc::new(ChannelSink { channel: on_message, forward: Mutex::new(None) });
    let session = Session::open(options, sink.clone()).map_err(message)?;
    let id = sessions.next.fetch_add(1, Ordering::SeqCst) + 1;
    sessions.open.lock().map_err(|e| e.to_string())?.insert(id, (session, sink));
    Ok(id)
}

/// While a plan runs, keys that arrive with no performance owning the
/// keyboard (between steps) must not reach the shell: they would be typed for
/// real, and the dirty line would stop the next step arming.
fn held_during_run(session: &Session, sink: &ChannelSink) -> bool {
    let running = locked(&sink.forward).is_some();
    running && !session.snapshot().is_some_and(|s| s.owns_input && s.state != ExecutionState::Complete)
}

#[tauri::command]
fn terminal_write(id: u32, data: String, sessions: State<'_, Arc<Sessions>>) -> Result<(), String> {
    let (session, sink) = sessions.get(id)?;
    if held_during_run(&session, &sink) && !is_terminal_report(data.as_bytes()) {
        return Err(message(CoreError::InputOwned));
    }
    session.write_input(data.as_bytes()).map_err(message)
}

#[tauri::command]
fn terminal_key(id: u32, chord: KeyChord, sessions: State<'_, Arc<Sessions>>) -> Result<(), String> {
    let (session, sink) = sessions.get(id)?;
    if held_during_run(&session, &sink) {
        return Ok(());
    }
    session.key(&chord).map_err(message)
}

#[tauri::command]
fn terminal_resize(id: u32, size: TerminalSize, sessions: State<'_, Arc<Sessions>>) -> Result<(), String> {
    sessions.get(id)?.0.resize(size).map_err(message)
}

#[tauri::command]
fn terminal_close(id: u32, sessions: State<'_, Arc<Sessions>>) -> Result<(), String> {
    if let Some((session, _)) = sessions.open.lock().map_err(|e| e.to_string())?.remove(&id) {
        session.close();
    }
    Ok(())
}

#[tauri::command]
fn performance_arm(
    id: u32,
    source: ScriptSource,
    config: PerformanceConfig,
    sessions: State<'_, Arc<Sessions>>,
) -> Result<PerformanceSnapshot, String> {
    let (session, _) = sessions.get(id)?;
    let script = match source {
        ScriptSource::SafeDemo => demo::safe_demo(session.shell_kind()),
        ScriptSource::OperatorCommands { commands } => StagedScript {
            steps: commands
                .into_iter()
                .map(|c| c.trim().to_owned())
                .filter(|c| !c.is_empty())
                .enumerate()
                .map(|(i, command)| StagedStep {
                    id: format!("step-{}", i + 1),
                    title: command.clone(),
                    command,
                    mode: None,
                    submit: None,
                    answers: None,
                })
                .collect(),
        },
    };
    session.arm(script, config).map_err(message)
}

#[tauri::command]
fn performance_disarm(id: u32, sessions: State<'_, Arc<Sessions>>) -> Result<(), String> {
    sessions.get(id)?.0.disarm();
    Ok(())
}

#[tauri::command]
fn performance_pause(id: u32, sessions: State<'_, Arc<Sessions>>) -> Result<(), String> {
    sessions.get(id)?.0.pause();
    Ok(())
}

#[tauri::command]
fn performance_resume(id: u32, sessions: State<'_, Arc<Sessions>>) -> Result<(), String> {
    sessions.get(id)?.0.resume();
    Ok(())
}

// ---- The plan workspace ----------------------------------------------------

fn with_workspace<T>(
    plans: &Plans,
    f: impl FnOnce(&mut Workspace) -> Result<T, String>,
) -> Result<WorkspaceView, String> {
    let mut guard = locked(&plans.workspace);
    let w = guard.as_mut().ok_or("there is no plan open")?;
    f(w)?;
    // Any change to the draft makes an earlier seal stale.
    *locked(&plans.sealed) = None;
    Ok(w.view())
}

#[tauri::command]
async fn agents_list() -> Result<Vec<AgentInfo>, String> {
    tauri::async_runtime::spawn_blocking(detect_all).await.map_err(|e| e.to_string())
}

fn handle(kind: AgentKind) -> Result<AgentHandle, String> {
    let info = detect_all()
        .into_iter()
        .find(|i| i.kind == kind)
        .ok_or_else(|| format!("{kind:?} is not a known agent"))?;
    match (info.path, info.version) {
        (Some(path), version) => Ok(AgentHandle {
            kind,
            program: PathBuf::from(path),
            version: version.unwrap_or_else(|| "unknown".into()),
        }),
        (None, _) => Err(format!("{} is not installed", info.name)),
    }
}

fn agents(runner: &ProcessRunner) -> Agents<'_, ProcessRunner> {
    Agents { runner, scratch: std::env::temp_dir(), max_repairs: 2 }
}

#[tauri::command]
fn workspace_view(plans: State<'_, Arc<Plans>>) -> Option<WorkspaceView> {
    locked(&plans.workspace).as_ref().map(Workspace::view)
}

#[tauri::command]
fn workspace_open(text: String, plans: State<'_, Arc<Plans>>) -> Result<WorkspaceView, String> {
    let w = Workspace::open(&text).map_err(|e| e.to_string())?;
    let view = w.view();
    *locked(&plans.workspace) = Some(w);
    *locked(&plans.sealed) = None;
    *locked(&plans.failed) = None;
    Ok(view)
}

/// Ask an agent for a plan. This sends the task and the pasted context,
/// redacted, to the agent the operator chose: it is their request.
#[tauri::command]
async fn workspace_propose(
    task: String,
    agent: AgentKind,
    context: String,
    plans: State<'_, Arc<Plans>>,
) -> Result<WorkspaceView, String> {
    let plans = plans.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let handle = handle(agent)?;
        let items = if context.trim().is_empty() {
            Vec::new()
        } else {
            vec![ContextItem::Text { label: "pasted".into(), text: context }]
        };
        let prepared = prepare(&items).map_err(|e| e.to_string())?;
        let runner = ProcessRunner::default();
        let w = Workspace::propose(&agents(&runner), &handle, &task, &prepared, &fingerprint::now_rfc3339())
            .map_err(|e| e.to_string())?;
        let view = w.view();
        *locked(&plans.workspace) = Some(w);
        *locked(&plans.sealed) = None;
        *locked(&plans.failed) = None;
        Ok(view)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn workspace_replace_step(step: Step, plans: State<'_, Arc<Plans>>) -> Result<WorkspaceView, String> {
    with_workspace(&plans, |w| w.replace_step(step, &fingerprint::now_rfc3339()).map_err(|e| e.to_string()))
}

#[tauri::command]
fn workspace_insert_step(
    after: Option<String>,
    step: Step,
    plans: State<'_, Arc<Plans>>,
) -> Result<WorkspaceView, String> {
    with_workspace(&plans, |w| {
        w.insert_step(after.as_deref(), step, &fingerprint::now_rfc3339()).map_err(|e| e.to_string())
    })
}

#[tauri::command]
fn workspace_remove_step(id: String, plans: State<'_, Arc<Plans>>) -> Result<WorkspaceView, String> {
    with_workspace(&plans, |w| w.remove_step(&id, &fingerprint::now_rfc3339()).map_err(|e| e.to_string()))
}

#[tauri::command]
fn workspace_move_step(
    id: String,
    earlier: bool,
    plans: State<'_, Arc<Plans>>,
) -> Result<WorkspaceView, String> {
    with_workspace(&plans, |w| {
        w.move_step(&id, earlier, &fingerprint::now_rfc3339()).map_err(|e| e.to_string())
    })
}

#[tauri::command]
fn workspace_note(
    text: String,
    step: Option<String>,
    plans: State<'_, Arc<Plans>>,
) -> Result<WorkspaceView, String> {
    let mut guard = locked(&plans.workspace);
    let w = guard.as_mut().ok_or("there is no plan open")?;
    w.note("You", &text, step.as_deref(), &fingerprint::now_rfc3339());
    Ok(w.view())
}

#[tauri::command]
async fn workspace_validate(plans: State<'_, Arc<Plans>>) -> Result<WorkspaceView, String> {
    let plans = plans.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        with_workspace(&plans, |w| {
            let options =
                Options { broker_available: keyjutsu_broker::broker_path().is_some(), ..Options::default() };
            w.validate(options, &fingerprint::now_rfc3339()).map_err(|e| e.to_string())
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Download and pin every artifact the draft needs. The source is
/// contacted now, before approval, so that nothing is downloaded at run time.
#[tauri::command]
async fn workspace_stage(plans: State<'_, Arc<Plans>>) -> Result<WorkspaceView, String> {
    let plans = plans.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        with_workspace(&plans, |w| {
            w.stage(&keyjutsu_core::artifacts::default_store(), &fingerprint::now_rfc3339())
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Run an agent request against the current draft, off the UI thread.
async fn agent_request(
    plans: &Arc<Plans>,
    agent: AgentKind,
    f: impl FnOnce(&mut Workspace, &Agents<'_, ProcessRunner>, &AgentHandle, &str) -> Result<(), String>
    + Send
    + 'static,
) -> Result<WorkspaceView, String> {
    let plans = plans.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let handle = handle(agent)?;
        let runner = ProcessRunner::default();
        let a = agents(&runner);
        // Work on a copy so the draft stays usable while the agent thinks.
        let mut copy = locked(&plans.workspace).clone().ok_or("there is no plan open")?;
        f(&mut copy, &a, &handle, &fingerprint::now_rfc3339())?;
        let view = copy.view();
        *locked(&plans.workspace) = Some(copy);
        *locked(&plans.sealed) = None;
        Ok(view)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn workspace_retry_step(
    agent: AgentKind,
    step: String,
    guidance: String,
    plans: State<'_, Arc<Plans>>,
) -> Result<WorkspaceView, String> {
    agent_request(plans.inner(), agent, move |w, a, h, at| {
        w.retry_step(a, h, &step, &guidance, None, at).map(|_| ()).map_err(|e| e.to_string())
    })
    .await
}

/// Ask the agent to fix the step the last run failed at, showing it what that
/// step printed. The replacement is a draft like any other: it is validated
/// and approved before it can run.
#[tauri::command]
async fn workspace_fix_failure(
    agent: AgentKind,
    guidance: String,
    plans: State<'_, Arc<Plans>>,
) -> Result<WorkspaceView, String> {
    let failed = locked(&plans.failed).clone().ok_or("the last run did not fail")?;
    agent_request(plans.inner(), agent, move |w, a, h, at| {
        w.retry_step(a, h, &failed.step, &guidance, Some(&failed.failure), at)
            .map(|_| ())
            .map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
async fn workspace_revise(
    agent: AgentKind,
    guidance: String,
    plans: State<'_, Arc<Plans>>,
) -> Result<WorkspaceView, String> {
    agent_request(plans.inner(), agent, move |w, a, h, at| {
        w.revise_plan(a, h, &guidance, at).map(|_| ()).map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
async fn workspace_review(agent: AgentKind, plans: State<'_, Arc<Plans>>) -> Result<WorkspaceView, String> {
    agent_request(plans.inner(), agent, move |w, a, h, at| w.review(a, h, at).map_err(|e| e.to_string()))
        .await
}

/// Approve and seal. A critical step needs its phrase in `confirmations`.
/// The snapshot is written where the CLI can run or recover it too.
#[tauri::command]
async fn workspace_approve(
    confirmations: BTreeMap<String, String>,
    plans: State<'_, Arc<Plans>>,
) -> Result<Sealed, String> {
    let plans = plans.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let w = locked(&plans.workspace).clone().ok_or("there is no plan open")?;
        let fp = fingerprint::collect(Some(w.plan()));
        let snapshot =
            w.approve(&confirmations, Some(fp), &fingerprint::now_rfc3339()).map_err(|e| e.to_string())?;
        let folder = run_folder(&snapshot);
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        let path = folder.join("snapshot.json");
        keyjutsu_core::approvals::record_approval(&open_store()?, &snapshot)?;
        std::fs::write(&path, snapshot.to_json()).map_err(|e| e.to_string())?;
        let sealed = Sealed {
            snapshot_hash: snapshot.snapshot_hash().to_owned(),
            path: path.display().to_string(),
            sealed_at: snapshot.sealed_at().to_owned(),
        };
        *locked(&plans.sealed) = Some((snapshot, path));
        Ok(sealed)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Run the sealed snapshot in terminal `id`. Returns at once; progress,
/// critical confirmations and the outcome arrive on `on_event`.
#[tauri::command]
fn plan_run(
    id: u32,
    config: PerformanceConfig,
    on_event: Channel<RunMessage>,
    sessions: State<'_, Arc<Sessions>>,
    plans: State<'_, Arc<Plans>>,
) -> Result<(), String> {
    let (session, sink) = sessions.get(id)?;
    let (snapshot, path) = locked(&plans.sealed).clone().ok_or("approve the plan first")?;
    // After a failure, carry on from where that run stopped: steps that
    // succeeded and have not changed are not run again, whatever the revision.
    // After a session boundary, carry on from the far side of it.
    let from = match locked(&plans.failed).clone() {
        Some(f) => Some(f.checkpoint),
        None => locked(&plans.waiting).clone().filter(|w| w.parent() == path.parent()),
    };
    let resume = match from {
        Some(f) => Some(keyjutsu_core::approvals::load_checkpoint(&open_store()?, &f)?),
        None => None,
    };
    // Crossing a boundary is the operator's decision, made on what KeyJutsu
    // found on this side of it, and made by typing RESUME: Rust compares it.
    let resume_gate = resume.as_ref().and_then(|c| c.boundary.as_ref()).map(|_| {
        let channel = on_event.clone();
        let answer = plans.answer.clone();
        let gate: keyjutsu_core::boundary::ResumeGate =
            Arc::new(move |notice: &keyjutsu_core::boundary::BoundaryNotice| {
                *locked(&answer.0) = None;
                let _ = channel.send(RunMessage::Resume { notice: notice.clone() });
                let (lock, ready) = &*answer;
                let mut slot = locked(lock);
                loop {
                    if let Some(a) = slot.take() {
                        return a.as_deref() == Some(RESUME_WORD);
                    }
                    slot = ready.wait(slot).unwrap_or_else(|e| e.into_inner());
                }
            });
        gate
    });
    let remember = plans.inner().clone();
    let checkpoint = path.with_file_name("snapshot.checkpoint.json");
    *locked(&plans.checkpoint) = Some(checkpoint.clone());
    let answer = plans.answer.clone();
    *locked(&answer.0) = None;
    let confirm_channel = on_event.clone();
    let gate: CriticalGate = Arc::new(move |c: &CriticalConfirmation| {
        let _ = confirm_channel.send(RunMessage::Confirm { confirmation: c.clone() });
        let (lock, ready) = &*answer;
        let mut slot = locked(lock);
        loop {
            if let Some(a) = slot.take() {
                return a;
            }
            slot = ready.wait(slot).unwrap_or_else(|e| e.into_inner());
        }
    });
    let needs_admin = snapshot
        .plan()
        .steps
        .iter()
        .any(|s| s.privilege == Some(keyjutsu_core::plan::model::Privilege::Administrator));
    let mut elevated_runner: Option<Arc<dyn keyjutsu_core::elevation::ElevatedRunner>> = None;
    if needs_admin && !keyjutsu_core::elevation::is_elevated() {
        let exe =
            keyjutsu_broker::broker_path().ok_or("keyjutsu-broker.exe is not installed next to KeyJutsu")?;
        elevated_runner = Some(Arc::new(keyjutsu_broker::launch(
            &exe,
            &path,
            snapshot.snapshot_hash(),
            &keyjutsu_core::artifacts::default_store(),
        )?));
    }
    let options = ExecuteOptions {
        elevated_runner,
        checkpoint_store: Some(Arc::new(open_store()?)),
        mode: Some(config.mode),
        base: config,
        checkpoint: Some(checkpoint.clone()),
        // A critical step approved more than an hour before it is reached is
        // asked for again, just before it runs; within the hour, the phrase
        // typed at approval stands. The executor judges the hour, step by step.
        critical_gate: Some(gate),
        resume_gate,
        ..ExecuteOptions::default()
    };
    std::thread::spawn(move || {
        // Record the repositories the plan works in, to tell its changes
        // from the operator's afterwards.
        let git_dir = path.with_file_name("git");
        // Where the shell really is: a profile may have changed folder.
        let start = session.shell_location().unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let baseline: Vec<(git::RepoState, PathBuf)> = git::repositories(&start, snapshot.plan())
            .iter()
            .enumerate()
            .filter_map(|(i, repo)| {
                let copies = git_dir.join(i.to_string());
                git::record(repo, Some(&copies)).ok().map(|r| (r, copies))
            })
            .collect();
        let (tx, rx) = channel();
        *locked(&sink.forward) = Some(tx);
        let started = fingerprint::now_rfc3339();
        let (outcome, finished_checkpoint) = execute(
            &Driver { session: &session, events: &rx },
            &snapshot,
            resume,
            &options,
            &fingerprint::now_rfc3339,
            &|event| {
                let _ = on_event.send(RunMessage::Execution { event });
            },
        );
        *locked(&sink.forward) = None;
        *locked(&remember.failed) = match &outcome {
            keyjutsu_core::execute::Outcome::Failed { step, expected, actual, output } => Some(Failed {
                step: step.clone(),
                failure: keyjutsu_core::agent::RunFailure {
                    expected: expected.clone(),
                    actual: actual.clone(),
                    output: output.clone(),
                },
                checkpoint: checkpoint.clone(),
            }),
            _ => None,
        };
        // Still on the near side of a boundary (it stopped at one, or the
        // resume was refused): the next run continues from here.
        *locked(&remember.waiting) = finished_checkpoint.boundary.is_some().then(|| checkpoint.clone());
        let git: Vec<git::RepoReport> =
            baseline.iter().filter_map(|(before, copies)| git::report(before, copies).ok()).collect();
        // Recorded in the encrypted history, as the CLI records its runs, so
        // a run that worked can become a Technique.
        let finished = fingerprint::now_rfc3339();
        let record = keyjutsu_core::history::SessionRecord {
            id: keyjutsu_core::store::new_id(&finished),
            started_at: started,
            finished_at: finished,
            task: snapshot.plan().title.clone().unwrap_or_else(|| snapshot.plan().task_id.clone()),
            agent: snapshot.plan().agent.clone(),
            snapshot: snapshot.to_json(),
            checkpoint: Some(finished_checkpoint),
            outcome: outcome.clone(),
            git: git.clone(),
        };
        let session = open_store()
            .and_then(|s| keyjutsu_core::history::save(&s, &record))
            .map(|()| record.id.clone())
            .ok();
        let _ = on_event.send(RunMessage::Done {
            outcome,
            snapshot: path.display().to_string(),
            checkpoint: checkpoint.display().to_string(),
            git,
            session,
        });
    });
    Ok(())
}

/// What the operator types to cross a session boundary.
const RESUME_WORD: &str = "RESUME";

/// The run waiting at a boundary: the one this window stopped, or, after a
/// restart, the most recent the app approved.
fn find_waiting(plans: &Plans) -> Result<Option<keyjutsu_core::boundary::Waiting>, String> {
    let mut candidates: Vec<PathBuf> = locked(&plans.waiting).clone().into_iter().collect();
    if let Ok(dirs) = std::fs::read_dir(keyjutsu_core::workspace::runs_root()) {
        candidates.extend(dirs.flatten().map(|d| d.path().join("snapshot.checkpoint.json")));
    }
    Ok(keyjutsu_core::boundary::find_waiting(&open_store()?, candidates))
}

/// A run waiting on the far side of a restart or other session boundary,
/// if there is one: after a Windows restart, the app offers to continue it.
#[tauri::command]
fn run_waiting(plans: State<'_, Arc<Plans>>) -> Result<Option<WaitingRun>, String> {
    Ok(find_waiting(&plans)?.and_then(|w| {
        let plan = w.snapshot.plan();
        let wait = w.wait()?;
        Some(WaitingRun {
            title: plan.title.clone().unwrap_or_else(|| plan.task_id.clone()),
            after_phase: wait.after_phase.clone(),
            boundary: wait.kind,
            recorded_at: wait.recorded_at.clone(),
        })
    }))
}

/// Open the waiting run to continue it. Nothing runs: the next run resumes
/// from its checkpoint, and asks before crossing the boundary.
#[tauri::command]
fn run_waiting_open(plans: State<'_, Arc<Plans>>) -> Result<WaitingOpened, String> {
    let keyjutsu_core::boundary::Waiting {
        snapshot, snapshot_path: path, checkpoint_path: checkpoint, ..
    } = find_waiting(&plans)?.ok_or("no run is waiting to continue")?;
    let draft = keyjutsu_core::plan::parse::ValidPlan::revalidate(snapshot.plan().clone(), false)
        .map_err(|e| e.to_string())?;
    let task = snapshot.plan().title.clone().unwrap_or_else(|| snapshot.plan().task_id.clone());
    let w = Workspace::new(task, draft);
    let view = w.view();
    let sealed = Sealed {
        snapshot_hash: snapshot.snapshot_hash().to_owned(),
        path: path.display().to_string(),
        sealed_at: snapshot.sealed_at().to_owned(),
    };
    *locked(&plans.workspace) = Some(w);
    *locked(&plans.sealed) = Some((snapshot, path));
    *locked(&plans.failed) = None;
    *locked(&plans.checkpoint) = Some(checkpoint.clone());
    *locked(&plans.waiting) = Some(checkpoint);
    Ok(WaitingOpened { view, sealed })
}

/// Every recorded run, newest first.
#[tauri::command]
fn history_list() -> Result<Vec<keyjutsu_core::history::SessionSummary>, String> {
    let mut list = keyjutsu_core::history::list(&open_store()?)?;
    list.reverse();
    Ok(list)
}

#[tauri::command]
fn history_show(id: String) -> Result<keyjutsu_core::history::SessionRecord, String> {
    keyjutsu_core::history::load(&open_store()?, &id)
}

/// Make a completed run into a Technique. Each pair is a parameter's name and
/// the value in the run's plan that it stands for.
#[tauri::command]
fn technique_promote(
    session: String,
    name: String,
    description: String,
    params: Vec<(String, String)>,
) -> Result<keyjutsu_core::technique::Technique, String> {
    let store = open_store()?;
    let record = keyjutsu_core::history::load(&store, &session)?;
    let promote: Vec<keyjutsu_core::technique::Promote<'_>> = params
        .iter()
        .map(|(n, v)| keyjutsu_core::technique::Promote { name: n, description: "", value: v, pattern: None })
        .collect();
    let t = keyjutsu_core::technique::promote(
        &record,
        &name,
        &description,
        &promote,
        &fingerprint::now_rfc3339(),
    )?;
    keyjutsu_core::technique::save(&store, &t)?;
    Ok(t)
}

#[tauri::command]
fn technique_list() -> Result<Vec<keyjutsu_core::technique::Technique>, String> {
    keyjutsu_core::technique::list(&open_store()?)
}

#[tauri::command]
fn technique_use(
    id: String,
    values: BTreeMap<String, String>,
    plans: State<'_, Arc<Plans>>,
) -> Result<keyjutsu_core::ipc::TechniqueDraft, String> {
    let t = keyjutsu_core::technique::latest(&open_store()?, &id)?;
    let draft = keyjutsu_core::technique::instantiate(&t, &values)?;
    let fit = keyjutsu_core::technique::fit(&t, &draft, &fingerprint::collect(Some(draft.plan())));
    let w = Workspace::open(&draft.to_json()).map_err(|e| e.to_string())?;
    let view = w.view();
    *locked(&plans.workspace) = Some(w);
    *locked(&plans.sealed) = None;
    *locked(&plans.failed) = None;
    Ok(keyjutsu_core::ipc::TechniqueDraft { view, fit })
}

/// The operator's answer to a critical confirmation: what they typed, or
/// nothing to decline. The executor compares it with the phrase.
#[tauri::command]
fn plan_confirm(typed: Option<String>, plans: State<'_, Arc<Plans>>) {
    let (lock, ready) = &*plans.answer;
    *locked(lock) = Some(typed);
    ready.notify_all();
}

fn last_run(plans: &Plans) -> Result<(ApprovedSnapshot, Checkpoint, PathBuf), String> {
    let (snapshot, _) = locked(&plans.sealed).clone().ok_or("no plan has run")?;
    let path = locked(&plans.checkpoint).clone().ok_or("no plan has run")?;
    let checkpoint = keyjutsu_core::approvals::load_checkpoint(&open_store()?, &path)?;
    Ok((snapshot, checkpoint, path))
}

/// The encrypted store, where approvals and checkpoints are recorded.
fn open_store() -> Result<keyjutsu_core::store::Store, String> {
    keyjutsu_core::store::Store::open(&keyjutsu_core::store::default_root())
}

#[tauri::command]
fn recovery_plan(plans: State<'_, Arc<Plans>>) -> Result<Vec<RecoveryItem>, String> {
    let (snapshot, checkpoint, _) = last_run(&plans)?;
    plan_recovery(&snapshot, &checkpoint, &[])
}

/// Carry out the recovery plan, in terminal `id` for any recovery commands.
#[tauri::command]
async fn recovery_run(
    id: u32,
    sessions: State<'_, Arc<Sessions>>,
    plans: State<'_, Arc<Plans>>,
) -> Result<Vec<RecoveryResult>, String> {
    let (snapshot, checkpoint, path) = last_run(&plans)?;
    let snapshot_file = locked(&plans.sealed).clone().map(|(_, p)| p).ok_or("no plan has run")?;
    let (session, sink) = sessions.get(id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let items = plan_recovery(&snapshot, &checkpoint, &[])?;
        // An Administrator step's recovery commands run in the elevation
        // broker, started now with one UAC prompt, never in this terminal.
        let broker = if keyjutsu_core::recovery::needs_broker(&snapshot, &items) {
            let exe = keyjutsu_broker::broker_path()
                .ok_or("keyjutsu-broker.exe is not installed next to KeyJutsu")?;
            Some(keyjutsu_broker::launch(
                &exe,
                &snapshot_file,
                snapshot.snapshot_hash(),
                &keyjutsu_core::artifacts::default_store(),
            )?)
        } else {
            None
        };
        let (tx, rx) = channel();
        *locked(&sink.forward) = Some(tx);
        let driver = Driver { session: &session, events: &rx };
        let results = recover(
            Some(&driver),
            broker.as_ref().map(|b| b as &dyn keyjutsu_core::elevation::ElevatedRunner),
            &snapshot,
            &checkpoint,
            &recovery_dir(&path),
            &items,
            &PerformanceConfig::default(),
            &|_| {},
        );
        *locked(&sink.forward) = None;
        Ok(results)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The folder after `--cwd`. Explorer passes a drive root as `"C:\"`, and
/// Windows' argument rules read the `\"` as an escaped quote, so it arrives
/// as `C:"`; that is put back as `C:\`.
fn cwd_argument(args: impl IntoIterator<Item = String>) -> Option<String> {
    let mut args = args.into_iter().skip_while(|a| a != "--cwd").skip(1);
    let dir = args.next()?;
    Some(match dir.strip_suffix('"') {
        Some(rest) => format!("{rest}\\"),
        None => dir,
    })
}

fn main() {
    // "Open KeyJutsu here" in Explorer passes the folder: terminals start there.
    if let Some(dir) = cwd_argument(std::env::args()) {
        let _ = std::env::set_current_dir(&dir);
        let _ = LAUNCH_FOLDER.set(PathBuf::from(dir));
    }
    let sessions = Arc::new(Sessions::default());
    let plans = Arc::new(Plans::default());
    // For checking the workspace on screen without driving the window: open
    // a plan file at start.
    if let Some(path) = std::env::var_os("KEYJUTSU_OPEN_PLAN")
        && let Ok(text) = std::fs::read_to_string(&path)
        && let Ok(w) = Workspace::open(&text)
    {
        *locked(&plans.workspace) = Some(w);
    }
    let on_exit = sessions.clone();
    let app = tauri::Builder::default()
        .manage(sessions)
        .manage(plans)
        .manage(Arc::new(Diagnostics::default()))
        .invoke_handler(tauri::generate_handler![
            readiness_scan,
            run_waiting,
            run_waiting_open,
            diagnostics_preview,
            diagnostics_save,
            terminal_profile,
            terminal_open,
            terminal_write,
            terminal_key,
            terminal_resize,
            terminal_close,
            performance_arm,
            performance_disarm,
            performance_pause,
            performance_resume,
            agents_list,
            workspace_view,
            workspace_open,
            workspace_propose,
            workspace_replace_step,
            workspace_insert_step,
            workspace_remove_step,
            workspace_move_step,
            workspace_note,
            workspace_validate,
            workspace_stage,
            workspace_retry_step,
            workspace_fix_failure,
            history_list,
            history_show,
            technique_promote,
            technique_list,
            technique_use,
            workspace_revise,
            workspace_review,
            workspace_approve,
            plan_run,
            plan_confirm,
            recovery_plan,
            recovery_run,
        ])
        .build(tauri::generate_context!());
    match app {
        Ok(app) => app.run(move |_, event| {
            if let tauri::RunEvent::Exit = event {
                // No shell outlives the window that showed it.
                if let Ok(open) = on_exit.open.lock() {
                    for (session, _) in open.values() {
                        session.close();
                    }
                }
            }
        }),
        Err(e) => {
            eprintln!("KeyJutsu could not start: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{cwd_argument, session_options};

    /// The folder given by "Open KeyJutsu here" is where the shell starts,
    /// not the home folder. Checked on a real shell, which reports where it
    /// is at its prompt.
    #[test]
    fn a_terminal_opens_in_the_folder_explorer_passed() {
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        use keyjutsu_core::headless::Collector;
        use keyjutsu_core::ipc::OpenRequest;
        use keyjutsu_core::terminal::{ProfileMode, ShellKind, TerminalSize};

        let folder = std::env::temp_dir().join(format!("keyjutsu-open-here-{}", std::process::id()));
        std::fs::create_dir_all(&folder).expect("scratch folder");
        let request = OpenRequest {
            shell: ShellKind::WindowsPowershell,
            profile: ProfileMode::Clean,
            size: TerminalSize { rows: 24, cols: 100 },
        };
        let mut options = session_options(&request, Some(folder.clone()));
        options.intercept_cursor_queries = true;
        let session = keyjutsu_core::Session::open(options, Arc::new(Collector::new())).expect("a shell");
        assert!(session.wait_ready(Duration::from_secs(30)), "no prompt");
        let deadline = Instant::now() + Duration::from_secs(10);
        while session.shell_location().is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        let here = session.shell_location();
        session.close();
        assert!(
            here.as_deref().is_some_and(|h| keyjutsu_core::git::same_path(h, &folder)),
            "the shell started in {here:?}, not {}",
            folder.display()
        );
        assert_eq!(session_options(&request, None).cwd, None, "no folder given: the home folder");
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn the_folder_explorer_passes_is_where_terminals_start() {
        assert_eq!(
            cwd_argument(args(&["app.exe", "--cwd", "D:/git/project"])).as_deref(),
            Some("D:/git/project")
        );
        assert_eq!(cwd_argument(args(&["app.exe"])), None);
        assert_eq!(cwd_argument(args(&["app.exe", "--cwd"])), None);
    }

    #[test]
    fn a_drive_root_survives_windows_quoting() {
        // What `"C:\"` becomes after Windows' argument rules.
        assert_eq!(cwd_argument(args(&["app.exe", "--cwd", "C:\""])).as_deref(), Some("C:\\"));
    }
}
