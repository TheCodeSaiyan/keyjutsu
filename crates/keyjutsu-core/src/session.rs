//! A terminal session: one real shell in a pseudo-console, and optionally a
//! performance driving its input.
//!
//! Both front ends use this type, so the rules about who may write to the
//! shell live here and nowhere else:
//!
//! - while a performance owns the input, raw writes from a front end are
//!   refused, apart from the automatic replies a terminal renderer sends
//!   (cursor position, focus, device attributes);
//! - a performance can only be armed at an idle, empty prompt, so staged text
//!   can never be appended to something already on the line;
//! - every action the engine asks for is carried out while its lock is held,
//!   so bytes reach the shell in exactly the order the engine decided.

use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use keyjutsu_execution::{
    Action, Bindings, ExecutionMode, Input, KeyClass, KeyInput, PerformanceConfig, PerformanceEngine,
    PerformanceSnapshot, StagedScript, StepOutcome, classify,
};
use keyjutsu_terminal::session::PtyParts;
use keyjutsu_terminal::utf8::Utf8Carry;
use keyjutsu_terminal::{
    KeyChord, MarkScanner, ProfileMode, PtySession, ScanItem, ShellKind, ShellLaunch, ShellMark, TerminalSize,
};
use serde::{Deserialize, Serialize};

use crate::CoreError;

/// Where session output and events go. Implemented by each front end.
///
/// Calls arrive on the session's own threads, some while its lock is held, so
/// an implementation must hand the data on (to a channel, a stream, a window)
/// and must not call back into the [`Session`] from inside these methods.
pub trait SessionSink: Send + Sync + 'static {
    /// Terminal output, already decoded, with KeyJutsu's marks removed.
    fn output(&self, text: &str);
    fn event(&self, event: SessionEvent);
    /// Answer to a cursor-position query, 1-based `(row, column)`. Only
    /// consulted when the session intercepts queries; see
    /// [`SessionOptions::intercept_cursor_queries`].
    fn cursor_position(&self) -> (u16, u16) {
        (1, 1)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum SessionEvent {
    /// The shell drew its first prompt with KeyJutsu's marks.
    ShellReady,
    Performance {
        snapshot: PerformanceSnapshot,
    },
    StepStarted {
        index: usize,
    },
    StepFinished {
        index: usize,
        outcome: StepOutcome,
    },
    /// The keyboard belongs to the operator again.
    Released,
    OverlayRequested,
    Exited {
        exit_code: Option<u32>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct SessionOptions {
    pub shell: ShellKind,
    #[serde(default)]
    pub profile: ProfileMode,
    #[serde(default)]
    #[ts(type = "string | null")]
    pub cwd: Option<PathBuf>,
    pub size: TerminalSize,
    /// Answer ConPTY's cursor-position queries from the sink instead of
    /// passing them to the renderer. xterm.js answers them itself; a headless
    /// session or a console pass-through must, or the shell never starts.
    #[serde(default)]
    pub intercept_cursor_queries: bool,
    #[serde(default)]
    pub bindings: Bindings,
}

impl SessionOptions {
    pub fn new(shell: ShellKind) -> Self {
        Self {
            shell,
            profile: ProfileMode::Detected,
            cwd: None,
            size: TerminalSize::default(),
            intercept_cursor_queries: false,
            bindings: Bindings::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellState {
    /// No prompt yet.
    Starting,
    /// At a prompt, as reported by the `B` mark.
    AtPrompt,
    /// A line was submitted and no prompt has come back yet.
    Busy,
}

struct Control {
    engine: Option<PerformanceEngine>,
    shell: ShellState,
    /// The operator has typed on the current prompt line.
    line_dirty: bool,
    ready: bool,
    exited: bool,
    /// Where the shell's last prompt said it was.
    location: Option<std::path::PathBuf>,
}

struct Inner {
    pty: PtySession,
    sink: Arc<dyn SessionSink>,
    bindings: Bindings,
    control: Mutex<Control>,
    ready_signal: Condvar,
    tick_generation: AtomicU64,
    /// What the shell printed, rendered as the terminal shows it, so a
    /// failed step's output can be read back: by the operator, and by the
    /// agent asked to fix it. A lock of its own: output arrives constantly
    /// and must not wait on the performance.
    recent: Mutex<crate::transcript::Transcript>,
}

/// A running shell session. Cheap to clone; all clones refer to one shell.
#[derive(Clone)]
pub struct Session {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session").field("pty", &self.inner.pty).finish_non_exhaustive()
    }
}

impl Session {
    pub fn open(options: SessionOptions, sink: Arc<dyn SessionSink>) -> Result<Self, CoreError> {
        options.bindings.validate().map_err(|e| CoreError::Refused(e.to_string()))?;
        let mut launch = ShellLaunch::for_kind(options.shell)?.with_profile(options.profile);
        if let Some(cwd) = &options.cwd {
            launch = launch.with_cwd(cwd);
        }
        let scanner =
            MarkScanner::new(launch.nonce.clone()).intercept_cursor_queries(options.intercept_cursor_queries);
        let PtyParts { session: pty, reader, mut child } = PtySession::spawn(&launch, options.size)?;

        let inner = Arc::new(Inner {
            pty,
            sink,
            bindings: options.bindings,
            control: Mutex::new(Control {
                engine: None,
                shell: ShellState::Starting,
                line_dirty: false,
                ready: false,
                exited: false,
                location: None,
            }),
            ready_signal: Condvar::new(),
            tick_generation: AtomicU64::new(0),
            recent: Mutex::new(crate::transcript::Transcript::new(options.size.rows, options.size.cols)),
        });

        let reading = inner.clone();
        std::thread::Builder::new()
            .name("keyjutsu-pty-reader".into())
            .spawn(move || reading.read_loop(reader, scanner))
            .map_err(|e| CoreError::Io(e.to_string()))?;

        let waiting = inner.clone();
        std::thread::Builder::new()
            .name("keyjutsu-pty-waiter".into())
            .spawn(move || {
                let exit_code = child.wait().ok().map(|s| s.exit_code());
                waiting.on_exit(exit_code);
            })
            .map_err(|e| CoreError::Io(e.to_string()))?;

        Ok(Self { inner })
    }

    pub fn shell_kind(&self) -> ShellKind {
        self.inner.pty.kind()
    }

    /// The folder the shell was in at its last prompt: where the next command
    /// runs. A profile that changes folder is reflected here.
    pub fn shell_location(&self) -> Option<std::path::PathBuf> {
        self.inner.lock().location.clone()
    }

    /// A position in the shell's output, to ask later what was printed since.
    pub fn output_mark(&self) -> usize {
        self.inner.recent.lock().map(|mut r| r.mark()).unwrap_or(0)
    }

    /// The terminal's size as the shell has it, and what redraws its screen
    /// as it is now: what a recording starts from (ADR 0021).
    pub fn screen(&self) -> (TerminalSize, String) {
        self.inner
            .recent
            .lock()
            .map(|r| {
                let (rows, cols) = r.size();
                (TerminalSize { rows, cols }, r.formatted())
            })
            .unwrap_or((TerminalSize::default(), String::new()))
    }

    /// What the shell printed since `mark`, as the terminal shows it: from
    /// the line the cursor was on, with a line the shell redrew read once.
    /// No more than the last `max_chars` characters of it.
    pub fn output_since(&self, mark: usize, max_chars: usize) -> String {
        self.inner.recent.lock().map(|mut r| r.since(mark, max_chars)).unwrap_or_default()
    }

    /// The shell's process id, which tells one shell from the next across a
    /// shell restart.
    pub fn shell_pid(&self) -> Option<u32> {
        self.inner.pty.process_id()
    }

    /// Block until the shell's first prompt, or until `timeout`. Returns
    /// whether the shell became ready. A shell whose profile replaces the
    /// prompt after KeyJutsu's wrapper is installed never becomes ready, and
    /// that is the signal to offer the clean profile.
    pub fn wait_ready(&self, timeout: Duration) -> bool {
        let control = self.inner.lock();
        let (control, _) = self
            .inner
            .ready_signal
            .wait_timeout_while(control, timeout, |c| !c.ready && !c.exited)
            .unwrap_or_else(|e| e.into_inner());
        control.ready
    }

    /// Block until the shell is idle at an empty prompt, or until `timeout`.
    /// Woken by the shell's own prompt marks, not by polling.
    pub fn wait_for_prompt(&self, timeout: Duration) -> bool {
        let idle = |c: &Control| c.shell == ShellState::AtPrompt && !c.line_dirty;
        let control = self.inner.lock();
        let (control, _) = self
            .inner
            .ready_signal
            .wait_timeout_while(control, timeout, |c| !idle(c) && !c.exited)
            .unwrap_or_else(|e| e.into_inner());
        idle(&control)
    }

    /// Raw input from a front end: bytes the renderer produced.
    pub fn write_input(&self, bytes: &[u8]) -> Result<(), CoreError> {
        let mut control = self.inner.lock();
        let owned = control.engine.as_ref().is_some_and(PerformanceEngine::owns_input);
        if owned {
            if is_terminal_report(bytes) {
                return self.inner.write(bytes);
            }
            return Err(CoreError::InputOwned);
        }
        if !is_terminal_report(bytes) {
            note_operator_input(&mut control, bytes);
        }
        self.inner.write(bytes)
    }

    /// A physical key. While a performance owns the keyboard the engine
    /// decides what it means; otherwise it is typed into the shell.
    pub fn key(&self, chord: &KeyChord) -> Result<(), CoreError> {
        let mut control = self.inner.lock();
        let class = classify(chord, &self.inner.bindings);
        if let Some(engine) = control.engine.as_mut()
            && (engine.owns_input() || class == KeyClass::HardDisarm)
        {
            let actions = engine.handle(Input::Key(KeyInput { class, bytes: chord.encode_vt() }));
            self.inner.apply(&mut control, actions);
            return Ok(());
        }
        if matches!(class, KeyClass::HardDisarm | KeyClass::Overlay) {
            // Special chords are never typed into the shell, armed or not.
            return Ok(());
        }
        match chord.encode_vt() {
            Some(bytes) => {
                note_operator_input(&mut control, &bytes);
                self.inner.write(&bytes)
            }
            None => Ok(()),
        }
    }

    /// Arm a performance of `script`.
    pub fn arm(
        &self,
        script: StagedScript,
        config: PerformanceConfig,
    ) -> Result<PerformanceSnapshot, CoreError> {
        let mut control = self.inner.lock();
        if control.exited {
            return Err(CoreError::Refused("the shell has exited".into()));
        }
        // A finished performance still holds the keyboard (so mashing past the
        // end types nothing); the next step of a plan may take over from it.
        // Anything still in progress may not be replaced.
        if control
            .engine
            .as_ref()
            .is_some_and(|e| e.owns_input() && e.state() != keyjutsu_execution::ExecutionState::Complete)
        {
            return Err(CoreError::Refused("a performance is already armed".into()));
        }
        if !control.ready {
            return Err(CoreError::Refused(
                "the shell has not reported a KeyJutsu prompt; its profile may be replacing the prompt"
                    .into(),
            ));
        }
        if control.shell != ShellState::AtPrompt {
            return Err(CoreError::Refused("the shell is busy; wait for the prompt".into()));
        }
        if control.line_dirty {
            return Err(CoreError::Refused("the command line is not empty; clear it before arming".into()));
        }
        let mut engine =
            PerformanceEngine::new(script, config).map_err(|e| CoreError::Refused(e.to_string()))?;
        let mut actions = engine.handle(Input::Arm);
        // Direct and user-input steps have no first keystroke to wait for,
        // except one that asks the operator: the engine waits for Enter.
        if matches!(engine.snapshot().step_mode, ExecutionMode::Direct | ExecutionMode::UserInput) {
            actions.extend(engine.handle(Input::Start));
        }
        control.engine = Some(engine);
        self.inner.apply(&mut control, actions);
        control.engine.as_ref().map(PerformanceEngine::snapshot).ok_or(CoreError::InputOwned)
    }

    pub fn disarm(&self) {
        self.inner.engine_input(Input::Disarm);
    }

    pub fn pause(&self) {
        self.inner.engine_input(Input::Pause);
    }

    pub fn resume(&self) {
        self.inner.engine_input(Input::Resume);
    }

    pub fn snapshot(&self) -> Option<PerformanceSnapshot> {
        self.inner.lock().engine.as_ref().map(PerformanceEngine::snapshot)
    }

    pub fn resize(&self, size: TerminalSize) -> Result<(), CoreError> {
        self.inner.pty.resize(size)?;
        if let Ok(mut recent) = self.inner.recent.lock() {
            recent.resize(size.rows, size.cols);
        }
        Ok(())
    }

    /// End the shell. Output already produced is still delivered.
    pub fn close(&self) {
        self.inner.engine_input(Input::Disarm);
        let _ = self.inner.pty.kill();
    }
}

/// Track what the operator does to the prompt line, so a performance is
/// never armed on top of half-typed text.
fn note_operator_input(control: &mut Control, bytes: &[u8]) {
    if bytes.contains(&b'\r') {
        control.line_dirty = false;
        if control.shell == ShellState::AtPrompt {
            control.shell = ShellState::Busy;
        }
    } else if control.shell == ShellState::AtPrompt {
        control.line_dirty = true;
    }
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, Control> {
        // A poisoned lock means a thread panicked mid-update. The state is
        // still the last consistent one the engine recorded, so carry on
        // rather than take the whole session down with it.
        self.control.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn write(&self, bytes: &[u8]) -> Result<(), CoreError> {
        Ok(self.pty.write(bytes)?)
    }

    fn engine_input(self: &Arc<Self>, input: Input) {
        let mut control = self.lock();
        if let Some(engine) = control.engine.as_mut() {
            let actions = engine.handle(input);
            self.apply(&mut control, actions);
        }
    }

    fn apply(self: &Arc<Self>, control: &mut Control, actions: Vec<Action>) {
        if actions.is_empty() {
            return;
        }
        let mut released = false;
        for action in actions {
            match action {
                Action::Write(bytes) => {
                    if bytes.contains(&b'\r') {
                        control.shell = ShellState::Busy;
                    }
                    if self.pty.write(&bytes).is_err() {
                        // The shell is gone; the waiter thread reports the exit.
                        break;
                    }
                }
                Action::ScheduleTick(delay) => {
                    let generation = self.tick_generation.fetch_add(1, Ordering::SeqCst) + 1;
                    let inner = self.clone();
                    let _ = std::thread::Builder::new().name("keyjutsu-tick".into()).spawn(move || {
                        std::thread::sleep(delay);
                        inner.tick(generation);
                    });
                }
                Action::CancelTicks => {
                    self.tick_generation.fetch_add(1, Ordering::SeqCst);
                }
                Action::StepStarted { index } => self.sink.event(SessionEvent::StepStarted { index }),
                Action::StepFinished { index, outcome } => {
                    self.sink.event(SessionEvent::StepFinished { index, outcome })
                }
                // Held back until the final snapshot has gone out, so a front
                // end reacting to the release sees the state that caused it.
                Action::Released => released = true,
                Action::OverlayRequested => self.sink.event(SessionEvent::OverlayRequested),
                Action::StateChanged { .. } => {}
            }
        }
        if let Some(engine) = control.engine.as_ref() {
            self.sink.event(SessionEvent::Performance { snapshot: engine.snapshot() });
        }
        if released {
            self.sink.event(SessionEvent::Released);
        }
    }

    fn tick(self: &Arc<Self>, generation: u64) {
        let mut control = self.lock();
        // Checked under the lock, so a tick cancelled by a pause or disarm
        // that happened while it slept cannot slip through.
        if self.tick_generation.load(Ordering::SeqCst) != generation {
            return;
        }
        if let Some(engine) = control.engine.as_mut() {
            let actions = engine.handle(Input::Tick);
            self.apply(&mut control, actions);
        }
    }

    fn read_loop(self: Arc<Self>, mut reader: Box<dyn Read + Send>, mut scanner: MarkScanner) {
        let mut decoder = Utf8Carry::new();
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let n = match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            for item in scanner.feed(&buf[..n]) {
                self.scanned(item, &mut decoder);
            }
        }
        for item in scanner.finish() {
            self.scanned(item, &mut decoder);
        }
    }

    fn scanned(self: &Arc<Self>, item: ScanItem, decoder: &mut Utf8Carry) {
        match item {
            ScanItem::Output(bytes) => {
                let text = decoder.decode(&bytes);
                if !text.is_empty() {
                    if let Ok(mut recent) = self.recent.lock() {
                        recent.feed(&text);
                    }
                    self.sink.output(&text);
                }
            }
            ScanItem::CursorQuery => {
                let (row, col) = self.sink.cursor_position();
                let _ = self.pty.write(format!("\x1b[{row};{col}R").as_bytes());
            }
            ScanItem::Mark(mark) => self.mark(mark),
            ScanItem::Location(path) => self.lock().location = Some(std::path::PathBuf::from(path)),
        }
    }

    fn mark(self: &Arc<Self>, mark: ShellMark) {
        let mut control = self.lock();
        if mark == ShellMark::CommandStart {
            control.shell = ShellState::AtPrompt;
            control.line_dirty = false;
            self.ready_signal.notify_all();
            if !control.ready {
                control.ready = true;
                self.sink.event(SessionEvent::ShellReady);
            }
        }
        if let Some(engine) = control.engine.as_mut() {
            let actions = engine.handle(Input::Shell(mark));
            self.apply(&mut control, actions);
        }
    }

    fn on_exit(self: &Arc<Self>, exit_code: Option<u32>) {
        {
            let mut control = self.lock();
            control.exited = true;
            self.ready_signal.notify_all();
            if let Some(engine) = control.engine.as_mut() {
                let actions = engine.handle(Input::ShellExited);
                self.apply(&mut control, actions);
            }
        }
        // Closing the pseudo-console ends the output stream once drained, so
        // the reader thread finishes too.
        self.pty.close();
        self.sink.event(SessionEvent::Exited { exit_code });
    }
}

/// Replies a terminal renderer sends on its own, as opposed to keys the user
/// pressed: cursor position reports, focus in and out, and device
/// attributes. Several may arrive in one write.
pub fn is_terminal_report(bytes: &[u8]) -> bool {
    let mut rest = bytes;
    if rest.is_empty() {
        return false;
    }
    while !rest.is_empty() {
        let Some(after) = rest.strip_prefix(b"\x1b[") else { return false };
        let (body, final_byte) = match after.iter().position(|b| (0x40..=0x7e).contains(b)) {
            Some(i) => (&after[..i], after[i]),
            None => return false,
        };
        let digits_and_semicolons = |s: &[u8]| s.iter().all(|b| b.is_ascii_digit() || *b == b';');
        let ok = match final_byte {
            b'I' | b'O' => body.is_empty(),
            b'R' => !body.is_empty() && digits_and_semicolons(body),
            b'c' => matches!(body.first(), Some(b'?' | b'>')) && digits_and_semicolons(&body[1..]),
            _ => false,
        };
        if !ok {
            return false;
        }
        rest = &after[body.len() + 1..];
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_renderer_replies_and_nothing_else() {
        for ok in
            [&b"\x1b[I"[..], b"\x1b[O", b"\x1b[12;40R", b"\x1b[?1;2c", b"\x1b[>0;276;0c", b"\x1b[I\x1b[3;1R"]
        {
            assert!(is_terminal_report(ok), "{ok:?}");
        }
        for bad in [
            &b""[..],
            b"a",
            b"\x1b[A",
            b"\x1b[3~",
            b"\x1b[Ia",
            b"Remove-Item x\r",
            b"\x1b[;R\r",
            b"\x1b[1;2R\x03",
            b"\x1b",
        ] {
            assert!(!is_terminal_report(bad), "{bad:?}");
        }
    }

    #[test]
    fn operator_typing_marks_the_line_dirty_until_it_is_submitted() {
        let mut c = Control {
            engine: None,
            shell: ShellState::AtPrompt,
            line_dirty: false,
            ready: true,
            exited: false,
            location: None,
        };
        note_operator_input(&mut c, b"dir");
        assert!(c.line_dirty);
        note_operator_input(&mut c, b"\r");
        assert!(!c.line_dirty);
        assert_eq!(c.shell, ShellState::Busy);
    }
}
