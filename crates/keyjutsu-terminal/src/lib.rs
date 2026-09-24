//! Real terminal sessions for KeyJutsu.
//!
//! Everything a user sees in a KeyJutsu terminal comes out of a Windows
//! pseudo-console (ConPTY) attached to a real shell process. This crate owns
//! that boundary: launching the shell with KeyJutsu's shell integration,
//! reading and writing the pseudo-console, and recognising the prompt marks the
//! integration emits so the execution engine can tell when a command has
//! finished without guessing from timers.
//!
//! Nothing here decides *what* to type. That is the execution engine's job.

pub mod keys;
pub mod marks;
pub mod platform;
pub mod profile;
pub mod session;
pub mod shell;
pub mod utf8;

pub use keys::{KeyChord, KeyName};
pub use marks::{MarkScanner, Nonce, ScanItem, ShellMark};
pub use session::{PtySession, TerminalSize};
pub use shell::{ProfileMode, ShellInfo, ShellKind, ShellLaunch};

/// Errors raised at the terminal boundary.
#[derive(Debug, thiserror::Error)]
pub enum TerminalError {
    #[error("pseudo-console failure: {0}")]
    Pty(String),
    #[error("shell `{0}` was not found")]
    ShellNotFound(String),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("could not obtain randomness for the session nonce: {0}")]
    Random(String),
}

pub type Result<T> = std::result::Result<T, TerminalError>;
