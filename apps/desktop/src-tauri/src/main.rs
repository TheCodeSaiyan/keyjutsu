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
use keyjutsu_core::ipc::{OpenRequest, RunMessage, ScriptSource, Sealed, TerminalMessage};
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
    /// The operator's answer to a critical confirmation: `Some(None)` declines.
    answer: Arc<(Mutex<Option<Option<String>>>, Condvar)>,
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

#[tauri::command]
fn terminal_profile() -> TerminalProfile {
    profile::detect_terminal_profile()
}

#[tauri::command]
fn terminal_open(
    request: OpenRequest,
    on_message: Channel<TerminalMessage>,
    sessions: State<'_, Arc<Sessions>>,
) -> Result<u32, String> {
    let mut options = SessionOptions::new(request.shell);
    options.profile = request.profile;
    options.size = request.size;
    // xterm.js answers ConPTY's cursor queries itself.
    options.intercept_cursor_queries = false;
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
            w.validate(Options::default(), &fingerprint::now_rfc3339()).map_err(|e| e.to_string())
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Download and pin every artifact the draft needs (§30). The source is
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
        w.retry_step(a, h, &step, &guidance, at).map(|_| ()).map_err(|e| e.to_string())
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
    let options = ExecuteOptions {
        mode: Some(config.mode),
        base: config,
        checkpoint: Some(checkpoint.clone()),
        // Approved in this window within the hour: the phrase typed at approval
        // stands. Older than that, it is asked for again before the step runs.
        critical_gate: keyjutsu_core::execute::needs_reconfirmation(
            snapshot.sealed_at(),
            fingerprint::now_secs(),
        )
        .then_some(gate),
        ..ExecuteOptions::default()
    };
    std::thread::spawn(move || {
        // Record the repositories the plan works in, to tell its changes
        // from the operator's afterwards (§31).
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
        let (outcome, _) = execute(
            &Driver { session: &session, events: &rx },
            &snapshot,
            None,
            &options,
            &fingerprint::now_rfc3339,
            &|event| {
                let _ = on_event.send(RunMessage::Execution { event });
            },
        );
        *locked(&sink.forward) = None;
        let git = baseline.iter().filter_map(|(before, copies)| git::report(before, copies).ok()).collect();
        let _ = on_event.send(RunMessage::Done {
            outcome,
            snapshot: path.display().to_string(),
            checkpoint: checkpoint.display().to_string(),
            git,
        });
    });
    Ok(())
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
    let checkpoint = Checkpoint::load(&path)?;
    Ok((snapshot, checkpoint, path))
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
    let (session, sink) = sessions.get(id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let items = plan_recovery(&snapshot, &checkpoint, &[])?;
        let (tx, rx) = channel();
        *locked(&sink.forward) = Some(tx);
        let driver = Driver { session: &session, events: &rx };
        let results = recover(
            Some(&driver),
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

fn main() {
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
        .invoke_handler(tauri::generate_handler![
            readiness_scan,
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
