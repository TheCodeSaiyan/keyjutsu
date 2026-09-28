//! Running a KeyJutsu session inside the console the CLI was started from.
//!
//! The session draws into the alternate screen buffer. ConPTY positions its
//! output absolutely, so it needs to know where the cursor starts; a clean
//! alternate screen with the cursor at the top-left makes that unambiguous and
//! leaves the user's own scrollback untouched when the session ends.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::{cursor, execute, terminal};
use keyjutsu_core::execution::{PerformanceConfig, StagedScript, StepOutcome};
use keyjutsu_core::terminal::{KeyChord, KeyName, TerminalSize};
use keyjutsu_core::{Session, SessionEvent, SessionOptions, SessionSink};

struct ConsoleSink {
    out: Mutex<std::io::Stdout>,
    exited: AtomicBool,
    overlay_requests: Mutex<u32>,
    outcomes: Mutex<Vec<(usize, StepOutcome)>>,
    released: AtomicBool,
    /// Events for an execution controller, when one is running.
    forward: Mutex<Option<Sender<SessionEvent>>>,
    /// What the terminal drew, when the run is recorded (ADR 0021).
    recorder: Option<Arc<keyjutsu_core::recording::Recorder>>,
}

impl SessionSink for ConsoleSink {
    fn output(&self, text: &str) {
        if let Some(r) = &self.recorder {
            r.output(text);
        }
        if let Ok(mut out) = self.out.lock() {
            let _ = out.write_all(text.as_bytes());
            let _ = out.flush();
        }
    }

    fn event(&self, event: SessionEvent) {
        if let Ok(forward) = self.forward.lock()
            && let Some(tx) = forward.as_ref()
        {
            let _ = tx.send(event.clone());
        }
        match event {
            SessionEvent::Exited { .. } => self.exited.store(true, Ordering::SeqCst),
            SessionEvent::OverlayRequested => {
                if let Ok(mut n) = self.overlay_requests.lock() {
                    *n += 1;
                }
            }
            SessionEvent::StepFinished { index, outcome } => {
                if let Ok(mut o) = self.outcomes.lock() {
                    o.push((index, outcome));
                }
            }
            SessionEvent::Released => self.released.store(true, Ordering::SeqCst),
            _ => {}
        }
    }

    fn cursor_position(&self) -> (u16, u16) {
        // Everything written so far has been flushed, so the console's own
        // cursor is where ConPTY expects it to be.
        cursor::position().map(|(col, row)| (row + 1, col + 1)).unwrap_or((1, 1))
    }
}

pub struct Performance {
    pub script: StagedScript,
    pub config: PerformanceConfig,
}

/// Drives a session from its own thread once the shell is ready, following
/// the session's events: how `keyjutsu run` executes a snapshot.
pub type Controller = Box<dyn FnOnce(Session, Receiver<SessionEvent>) + Send>;

/// A question put to the operator in this console while a controller drives
/// the session: how a critical step is confirmed again, just before it runs.
/// While one is open, keys go to the answer and nowhere else, never to the
/// shell.
#[derive(Clone, Default)]
pub struct Asker(Arc<Mutex<Option<Open>>>);

struct Open {
    typed: String,
    answer: Sender<Option<String>>,
    /// Show what is typed. Off when the question is put in the title bar.
    echo: bool,
}

/// Set the console window's title.
pub fn set_title(text: &str) {
    let _ = execute!(std::io::stdout(), terminal::SetTitle(text));
}

fn show(text: &str) {
    let mut out = std::io::stdout();
    let _ = out.write_all(text.replace('\n', "\r\n").as_bytes());
    let _ = out.flush();
}

impl Asker {
    /// Show `text` and wait for a line: what was typed on Enter, `None` on
    /// Esc or Ctrl+C.
    pub fn ask(&self, text: &str) -> Option<String> {
        let (answer, reply) = channel();
        // Open before it is shown: a key pressed the moment it appears
        // belongs to the answer.
        if let Ok(mut open) = self.0.lock() {
            *open = Some(Open { typed: String::new(), answer, echo: true });
        }
        show(text);
        reply.recv().ok().flatten()
    }

    /// As `ask`, with the question in the window's title bar and nothing in
    /// the console: what is typed is not shown.
    pub fn ask_in_title(&self, question: &str) -> Option<String> {
        let (answer, reply) = channel();
        if let Ok(mut open) = self.0.lock() {
            *open = Some(Open { typed: String::new(), answer, echo: false });
        }
        set_title(question);
        reply.recv().ok().flatten()
    }

    /// Take `key` for an open question. Returns whether it was taken.
    fn key(&self, key: &KeyEvent) -> bool {
        let Ok(mut guard) = self.0.lock() else { return false };
        let Some(open) = guard.as_mut() else { return false };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Enter => {
                if open.echo {
                    show("\n");
                }
                if let Some(o) = guard.take() {
                    let _ = o.answer.send(Some(o.typed));
                }
            }
            KeyCode::Esc | KeyCode::Char('c') if key.code == KeyCode::Esc || ctrl => {
                if open.echo {
                    show("\n");
                }
                if let Some(o) = guard.take() {
                    let _ = o.answer.send(None);
                }
            }
            KeyCode::Backspace => {
                if open.typed.pop().is_some() && open.echo {
                    show("\u{8} \u{8}");
                }
            }
            KeyCode::Char(c) if !ctrl => {
                open.typed.push(c);
                if open.echo {
                    show(&c.to_string());
                }
            }
            _ => {}
        }
        true
    }
}

pub struct RunSummary {
    pub outcomes: Vec<(usize, StepOutcome)>,
    pub armed: Option<Result<(), String>>,
}

struct RawModeGuard;

impl RawModeGuard {
    fn enter() -> std::io::Result<Self> {
        // Enables virtual-terminal processing on this console as a side effect.
        let _ = crossterm::ansi_support::supports_ansi();
        terminal::enable_raw_mode()?;
        execute!(
            std::io::stdout(),
            terminal::EnterAlternateScreen,
            terminal::Clear(terminal::ClearType::All),
            cursor::MoveTo(0, 0)
        )?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = execute!(std::io::stdout(), terminal::LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

/// Run a session in this console until its shell exits. With a performance,
/// it is armed as soon as the shell reaches its first prompt.
pub fn run(
    options: SessionOptions,
    performance: Option<Performance>,
    controller: Option<Controller>,
    asker: Option<Asker>,
) -> Result<RunSummary, String> {
    run_recorded(options, performance, controller, asker, None)
}

/// As [`run`], with what the terminal draws also given to `recorder`.
pub fn run_recorded(
    mut options: SessionOptions,
    performance: Option<Performance>,
    controller: Option<Controller>,
    asker: Option<Asker>,
    recorder: Option<Arc<keyjutsu_core::recording::Recorder>>,
) -> Result<RunSummary, String> {
    let (cols, rows) = terminal::size().map_err(|e| e.to_string())?;
    options.size = TerminalSize { rows, cols };
    options.intercept_cursor_queries = true;

    let guard = RawModeGuard::enter().map_err(|e| e.to_string())?;
    let sink = Arc::new(ConsoleSink {
        out: Mutex::new(std::io::stdout()),
        exited: AtomicBool::new(false),
        overlay_requests: Mutex::new(0),
        outcomes: Mutex::new(Vec::new()),
        released: AtomicBool::new(false),
        forward: Mutex::new(None),
        recorder,
    });
    let session = Session::open(options, sink.clone()).map_err(|e| e.to_string())?;

    // While a controller drives the session, keys pressed between its
    // performances must not reach the shell: they would be typed for real, and
    // the dirty line would then stop the next performance arming.
    let controlling = Arc::new(AtomicBool::new(controller.is_some()));
    if let Some(control) = controller {
        let (tx, rx) = channel();
        if let Ok(mut f) = sink.forward.lock() {
            *f = Some(tx);
        }
        if session.wait_ready(Duration::from_secs(30)) {
            let (s, done, after) = (session.clone(), controlling.clone(), session.clone());
            std::thread::spawn(move || {
                control(s, rx);
                done.store(false, Ordering::SeqCst);
                // Said only now that the controller has let go: said sooner,
                // an answer typed straight away would be swallowed. A finished
                // performance still holds the keyboard, so mashing past the
                // end types nothing into the shell; then only the disarm
                // chord gives it back, and the title says so.
                set_title(if after.snapshot().is_some() {
                    "KeyJutsu: the run has ended. Ctrl+Alt+Shift+K gives you the keyboard back."
                } else {
                    "KeyJutsu: the run has ended. Type exit to leave."
                });
            });
        } else {
            session.close();
            drop(guard);
            return Err("the shell never showed a KeyJutsu prompt; try --clean".to_owned());
        }
    }

    let mut armed = None;
    if let Some(p) = performance {
        let result = if session.wait_ready(Duration::from_secs(30)) {
            session.arm(p.script, p.config).map(|_| ()).map_err(|e| e.to_string())
        } else {
            Err("the shell never showed a KeyJutsu prompt; try --clean".to_owned())
        };
        if result.is_err() {
            session.close();
        }
        armed = Some(result);
    }

    let mut overlays_seen = 0;
    let mut paused = false;
    while !sink.exited.load(Ordering::SeqCst) {
        // A short poll so the loop notices the shell exiting. Input itself is
        // delivered as soon as it arrives.
        if !event::poll(Duration::from_millis(100)).unwrap_or(false) {
            continue;
        }
        match event::read() {
            Ok(Event::Key(key)) if key.kind != KeyEventKind::Release => {
                if asker.as_ref().is_some_and(|a| a.key(&key)) {
                    continue;
                }
                if controlling.load(Ordering::SeqCst) && session.snapshot().is_none() {
                    continue;
                }
                if let Some(chord) = chord_from(&key) {
                    let _ = session.key(&chord);
                }
            }
            Ok(Event::Resize(cols, rows)) => {
                let _ = session.resize(TerminalSize { rows, cols });
                // A recording is replayed at the size each part was drawn for.
                if let Some(r) = &sink.recorder {
                    r.resize(cols, rows);
                }
            }
            _ => {}
        }
        // The CLI has no overlay to draw: the overlay chord pauses and
        // resumes instead, invisibly.
        let requested = sink.overlay_requests.lock().map(|n| *n).unwrap_or(0);
        if requested != overlays_seen {
            overlays_seen = requested;
            paused = !paused;
            if paused { session.pause() } else { session.resume() }
        }
    }
    drop(guard);
    let outcomes = sink.outcomes.lock().map(|o| o.clone()).unwrap_or_default();
    Ok(RunSummary { outcomes, armed })
}

/// Translate crossterm's key event into KeyJutsu's front-end-neutral chord.
pub fn chord_from(key: &KeyEvent) -> Option<KeyChord> {
    let m = key.modifiers;
    let mut chord = KeyChord {
        key: KeyName::Other,
        ctrl: m.contains(KeyModifiers::CONTROL),
        alt: m.contains(KeyModifiers::ALT),
        shift: m.contains(KeyModifiers::SHIFT),
        meta: m.contains(KeyModifiers::SUPER),
    };
    chord.key = match key.code {
        KeyCode::Char(c) => KeyName::Char(c),
        KeyCode::Enter => KeyName::Enter,
        KeyCode::Tab => KeyName::Tab,
        KeyCode::BackTab => {
            chord.shift = true;
            KeyName::Tab
        }
        KeyCode::Backspace => KeyName::Backspace,
        KeyCode::Esc => KeyName::Escape,
        KeyCode::Up => KeyName::Up,
        KeyCode::Down => KeyName::Down,
        KeyCode::Left => KeyName::Left,
        KeyCode::Right => KeyName::Right,
        KeyCode::Home => KeyName::Home,
        KeyCode::End => KeyName::End,
        KeyCode::PageUp => KeyName::PageUp,
        KeyCode::PageDown => KeyName::PageDown,
        KeyCode::Insert => KeyName::Insert,
        KeyCode::Delete => KeyName::Delete,
        KeyCode::F(n) => KeyName::F(n),
        _ => return None,
    };
    Some(chord)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_the_disarm_chord_with_all_its_modifiers() {
        let key = KeyEvent::new(
            KeyCode::Char('K'),
            KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT,
        );
        let chord = chord_from(&key).unwrap();
        assert!(chord.ctrl && chord.alt && chord.shift);
        assert_eq!(chord.key, KeyName::Char('K'));
    }

    #[test]
    fn back_tab_is_shift_tab() {
        let chord = chord_from(&KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE)).unwrap();
        assert_eq!((chord.key, chord.shift), (KeyName::Tab, true));
    }
}
