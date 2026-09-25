//! The KeyJutsu application core.
//!
//! The desktop app and the `keyjutsu` CLI are both thin front ends over this
//! crate. Anything that decides what may reach a shell, or when, lives here
//! or below it, never in a front end. The desktop's React code in particular
//! can only ask; this crate answers.

pub mod demo;
pub mod execute;
pub mod fingerprint;
pub mod headless;
pub mod ipc;
pub mod readiness;
pub mod session;

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
