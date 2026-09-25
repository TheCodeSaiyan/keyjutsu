//! The KeyJutsu plan model.
//!
//! A plan is data, never prose: steps, the control flow between them, and the
//! conditions that choose between branches, in a versioned format checked
//! against `schemas/plan/v1`. This crate parses plans through the schema and
//! then checks what the schema cannot (cycles, dangling references,
//! conditions on steps that cannot have run yet), evaluates conditions
//! without guessing at missing facts, decides which step runs next, and says
//! what a change to a plan affects.
//!
//! It has no I/O. Facts come in through the [`condition::Facts`] trait.

pub mod approval;
pub mod canonical;
pub mod condition;
pub mod diff;
pub mod graph;
pub mod hash;
pub mod model;
pub mod parse;
pub mod version;
pub mod walk;

pub use approval::{Approval, ApprovalBook, ApprovedSnapshot, StepApproval, seal};
pub use condition::{Facts, KnownFacts, Missing, StepResult, Truth, evaluate};
pub use diff::{PlanDiff, StepChange, diff};
pub use graph::{PlanGraph, Problem};
pub use hash::{EnvironmentFingerprint, step_hashes};
pub use model::{Condition, Plan, Step};
pub use parse::{PlanError, ValidPlan, parse_plan, parse_proposal};
pub use walk::{Frontier, frontier};

/// "1 step", "2 steps": a count as a person would say it.
pub fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}
