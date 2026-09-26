//! Lite adapters for installed AI coding agents.
//!
//! An agent investigates and proposes; it never executes. KeyJutsu runs
//! each agent's CLI in its read-only mode, gives it a manifest-visible,
//! redacted context, and treats everything it returns as an untrusted
//! proposal that must pass the same gates as any other plan.

pub mod agents;
pub mod context;
pub mod detect;
pub mod extract;
pub mod prompt;
pub mod review;
pub mod session;

pub use agents::AgentKind;
pub use detect::{AgentInfo, detect_all};
pub use session::{
    AgentError, AgentHandle, Agents, ProcessRunner, Proposal, RunFailure, Runner, StepRevision,
};
