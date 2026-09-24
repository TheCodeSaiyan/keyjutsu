//! The KeyJutsu desktop app's Rust side.
//!
//! The React front end can only ask. Every command here hands the request to
//! keyjutsu-core, which decides whether it is allowed: raw input while a
//! performance owns the keyboard is refused there, arming is checked there,
//! and the demo script is built there rather than accepted from the window.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use keyjutsu_core::execution::{PerformanceConfig, PerformanceSnapshot, StagedScript, StagedStep};
use keyjutsu_core::ipc::{OpenRequest, ScriptSource, TerminalMessage};
use keyjutsu_core::readiness::{self, ReadinessReport};
use keyjutsu_core::terminal::profile::{self, TerminalProfile};
use keyjutsu_core::terminal::{KeyChord, TerminalSize};
use keyjutsu_core::{CoreError, Session, SessionEvent, SessionOptions, SessionSink, demo};
use tauri::State;
use tauri::ipc::Channel;

struct ChannelSink(Channel<TerminalMessage>);

impl SessionSink for ChannelSink {
    fn output(&self, text: &str) {
        let _ = self.0.send(TerminalMessage::Output { data: text.to_owned() });
    }

    fn event(&self, event: SessionEvent) {
        let _ = self.0.send(TerminalMessage::Event { event });
    }
}

#[derive(Default)]
struct Sessions {
    next: AtomicU32,
    open: Mutex<HashMap<u32, Session>>,
}

impl Sessions {
    fn get(&self, id: u32) -> Result<Session, String> {
        self.open
            .lock()
            .map_err(|e| e.to_string())?
            .get(&id)
            .cloned()
            .ok_or_else(|| format!("no terminal session {id}"))
    }
}

fn message(e: CoreError) -> String {
    match e {
        // A stable token the front end can recognise and ignore quietly.
        CoreError::InputOwned => "input_owned".into(),
        other => other.to_string(),
    }
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
    let session = Session::open(options, Arc::new(ChannelSink(on_message))).map_err(message)?;
    let id = sessions.next.fetch_add(1, Ordering::SeqCst) + 1;
    sessions.open.lock().map_err(|e| e.to_string())?.insert(id, session);
    Ok(id)
}

#[tauri::command]
fn terminal_write(id: u32, data: String, sessions: State<'_, Arc<Sessions>>) -> Result<(), String> {
    sessions.get(id)?.write_input(data.as_bytes()).map_err(message)
}

#[tauri::command]
fn terminal_key(id: u32, chord: KeyChord, sessions: State<'_, Arc<Sessions>>) -> Result<(), String> {
    sessions.get(id)?.key(&chord).map_err(message)
}

#[tauri::command]
fn terminal_resize(id: u32, size: TerminalSize, sessions: State<'_, Arc<Sessions>>) -> Result<(), String> {
    sessions.get(id)?.resize(size).map_err(message)
}

#[tauri::command]
fn terminal_close(id: u32, sessions: State<'_, Arc<Sessions>>) -> Result<(), String> {
    if let Some(session) = sessions.open.lock().map_err(|e| e.to_string())?.remove(&id) {
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
    let session = sessions.get(id)?;
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
                })
                .collect(),
        },
    };
    session.arm(script, config).map_err(message)
}

#[tauri::command]
fn performance_disarm(id: u32, sessions: State<'_, Arc<Sessions>>) -> Result<(), String> {
    sessions.get(id)?.disarm();
    Ok(())
}

#[tauri::command]
fn performance_pause(id: u32, sessions: State<'_, Arc<Sessions>>) -> Result<(), String> {
    sessions.get(id)?.pause();
    Ok(())
}

#[tauri::command]
fn performance_resume(id: u32, sessions: State<'_, Arc<Sessions>>) -> Result<(), String> {
    sessions.get(id)?.resume();
    Ok(())
}

fn main() {
    let sessions = Arc::new(Sessions::default());
    let on_exit = sessions.clone();
    let app = tauri::Builder::default()
        .manage(sessions)
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
        ])
        .build(tauri::generate_context!());
    match app {
        Ok(app) => app.run(move |_, event| {
            if let tauri::RunEvent::Exit = event {
                // No shell outlives the window that showed it.
                if let Ok(open) = on_exit.open.lock() {
                    for session in open.values() {
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
