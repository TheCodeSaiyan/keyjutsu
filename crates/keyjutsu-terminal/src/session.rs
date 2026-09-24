//! A shell running inside a Windows pseudo-console.
//!
//! This is the only place KeyJutsu writes to a shell. The visible terminal and
//! the process that executes commands are the same pseudo-console, so what the
//! user sees is what ran.

use std::io::{Read, Write};
use std::sync::Mutex;

use portable_pty::{Child, ChildKiller, MasterPty, PtySize, native_pty_system};
use serde::{Deserialize, Serialize};

use crate::shell::{ShellKind, ShellLaunch};
use crate::{Result, TerminalError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct TerminalSize {
    pub rows: u16,
    pub cols: u16,
}

impl Default for TerminalSize {
    fn default() -> Self {
        Self { rows: 30, cols: 120 }
    }
}

impl From<TerminalSize> for PtySize {
    fn from(s: TerminalSize) -> Self {
        PtySize { rows: s.rows.max(1), cols: s.cols.max(1), pixel_width: 0, pixel_height: 0 }
    }
}

/// The write side of a running shell. Reading and waiting are handed to the
/// caller as [`PtyParts`] because both block, and the caller decides which
/// threads they run on.
pub struct PtySession {
    kind: ShellKind,
    pid: Option<u32>,
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
}

impl std::fmt::Debug for PtySession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PtySession").field("kind", &self.kind).field("pid", &self.pid).finish()
    }
}

/// A freshly spawned session, plus the blocking halves: the output stream and
/// the child process to wait on.
pub struct PtyParts {
    pub session: PtySession,
    pub reader: Box<dyn Read + Send>,
    pub child: Box<dyn Child + Send + Sync>,
}

impl std::fmt::Debug for PtyParts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PtyParts").field("session", &self.session).finish_non_exhaustive()
    }
}

fn pty_err(e: impl std::fmt::Display) -> TerminalError {
    TerminalError::Pty(e.to_string())
}

impl PtySession {
    pub fn spawn(launch: &ShellLaunch, size: TerminalSize) -> Result<PtyParts> {
        crate::platform::restore_ctrl_c_for_children();
        let pair = native_pty_system().openpty(size.into()).map_err(pty_err)?;
        let child = pair.slave.spawn_command(launch.command_builder()).map_err(pty_err)?;
        // The pseudo-console holds its own handle to the slave side; keeping
        // ours open would stop the output stream ending when the shell exits.
        drop(pair.slave);
        let reader = pair.master.try_clone_reader().map_err(pty_err)?;
        let writer = pair.master.take_writer().map_err(pty_err)?;
        let session = PtySession {
            kind: launch.kind,
            pid: child.process_id(),
            killer: Mutex::new(child.clone_killer()),
            writer: Mutex::new(writer),
            master: Mutex::new(Some(pair.master)),
        };
        Ok(PtyParts { session, reader, child })
    }

    pub fn kind(&self) -> ShellKind {
        self.kind
    }

    pub fn process_id(&self) -> Option<u32> {
        self.pid
    }

    /// Send bytes to the shell exactly as a keyboard would.
    pub fn write(&self, bytes: &[u8]) -> Result<()> {
        let mut writer = self.writer.lock().map_err(pty_err)?;
        writer.write_all(bytes)?;
        writer.flush()?;
        Ok(())
    }

    pub fn resize(&self, size: TerminalSize) -> Result<()> {
        let master = self.master.lock().map_err(pty_err)?;
        match master.as_ref() {
            Some(m) => m.resize(size.into()).map_err(pty_err),
            None => Err(TerminalError::Pty("the pseudo-console is closed".into())),
        }
    }

    pub fn kill(&self) -> Result<()> {
        self.killer.lock().map_err(pty_err)?.kill()?;
        Ok(())
    }

    /// Close the pseudo-console. Its output stream ends once it has drained,
    /// which is what lets a reader thread finish after the shell has exited.
    pub fn close(&self) {
        if let Ok(mut master) = self.master.lock() {
            master.take();
        }
    }
}
