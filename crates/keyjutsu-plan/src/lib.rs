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

pub mod condition;
pub mod diff;
pub mod graph;
pub mod model;
pub mod parse;
pub mod version;
pub mod walk;

pub use condition::{Facts, KnownFacts, Missing, StepResult, Truth, evaluate};
pub use diff::{PlanDiff, StepChange, diff};
pub use graph::{PlanGraph, Problem};
pub use model::{Condition, Plan, Step};
pub use parse::{PlanError, ValidPlan, parse_plan, parse_proposal};
pub use walk::{Frontier, frontier};
