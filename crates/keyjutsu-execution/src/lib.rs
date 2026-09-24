//! Execution for KeyJutsu: the state machine and the Performance Mode engine.
//!
//! The engine decides what reaches the shell's input and when; it never
//! touches a terminal itself. `keyjutsu-core` connects it to a real
//! pseudo-console session.

pub mod cadence;
pub mod engine;
pub mod input;
pub mod script;
pub mod state;

pub use cadence::Cadence;
pub use engine::{Action, Input, PerformanceConfig, PerformanceEngine, PerformanceSnapshot, StepOutcome};
pub use input::{Bindings, KeyClass, KeyInput, classify};
pub use script::{AdvanceStyle, ExecutionMode, ScriptError, StagedScript, StagedStep, SubmitPolicy};
pub use state::ExecutionState;
