//! The KeyJutsu application core.
//!
//! The desktop app and the `keyjutsu` CLI are both thin front ends over this
//! crate. Anything that decides what may reach a shell, or when, lives here
//! or below it, never in a front end. The desktop's React code in particular
//! can only ask; this crate answers.

pub mod approvals;
pub mod artifacts;
pub mod boundary;
pub mod demo;
pub mod diagnostics;
pub mod dpapi;
pub mod elevation;
pub mod execute;
pub mod fingerprint;
pub mod git;
pub mod headless;
pub mod history;
pub mod ipc;
pub mod links;
pub mod readiness;
pub mod recovery;
pub mod runlock;
pub mod session;
pub mod store;
pub mod technique;
pub mod transcript;
pub mod workspace;

pub use keyjutsu_agent as agent;
pub use keyjutsu_execution as execution;
pub use keyjutsu_plan as plan;
pub use keyjutsu_terminal as terminal;
pub use keyjutsu_validation as validation;
pub use session::{Session, SessionEvent, SessionOptions, SessionSink};

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error(transparent)]
    Terminal(#[from] keyjutsu_terminal::TerminalError),
    /// A performance owns the keyboard; raw input is not accepted.
    #[error("a performance owns the terminal input")]
    InputOwned,
    #[error("refused: {0}")]
    Refused(String),
    #[error("i/o error: {0}")]
    Io(String),
}
