//! Turning text into a plan KeyJutsu can trust the shape of.
//!
//! Four gates, in order, each of which can only narrow what gets through:
//!
//! 1. **Size and syntax.** Over 1 MiB, or not JSON, is refused.
//! 2. **Version.** `schema_version` is read before anything else. A major or
//!    minor version KeyJutsu does not know is refused outright: a future
//!    format is never interpreted by guessing (§50).
//! 3. **Schema.** The document is checked against the JSON Schema compiled
//!    into KeyJutsu (the proposal schema for agent output), not a copy fetched
//!    from anywhere.
//! 4. **Structure.** The graph checks the schema cannot express: unknown step
//!    references, cycles, conditions on steps that cannot have run yet,
//!    phases, version-constraint syntax.
//!
//! The typed model is built between 3 and 4. It should never refuse something
//! the schema accepted; if it does, the two have drifted apart, which is a
//! bug, and it is reported as one rather than hidden.

use std::sync::LazyLock;

use jsonschema::{Retrieve, Uri, Validator};
use serde::Serialize;
use serde_json::Value;

use crate::graph::{PlanGraph, Problem, analyse};
use crate::model::Plan;

pub const MAX_PLAN_BYTES: usize = 1024 * 1024;
pub const SUPPORTED_VERSION: &str = "1.0";

const PLAN_SCHEMA: &str = include_str!("../../../schemas/plan/v1/plan.schema.json");
const PROPOSAL_SCHEMA: &str = include_str!("../../../schemas/plan/v1/proposal.schema.json");
const PLAN_SCHEMA_ID: &str = "urn:keyjutsu:schema:plan:1.0";

#[allow(clippy::expect_used)] // The schemas are compiled in; a broken one fails every test.
static PLAN_VALIDATOR: LazyLock<Validator> = LazyLock::new(|| {
    let schema: Value = serde_json::from_str(PLAN_SCHEMA).expect("plan schema is JSON");
    jsonschema::validator_for(&schema).expect("plan schema compiles")
});

/// Resolves the one external reference KeyJutsu's schemas make, the proposal
/// schema's reference to the plan schema, and refuses everything else. No
/// schema is ever fetched.
struct CompiledIn;

impl Retrieve for CompiledIn {
    fn retrieve(&self, uri: &Uri<String>) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        if uri.as_str() == PLAN_SCHEMA_ID {
            Ok(serde_json::from_str(PLAN_SCHEMA)?)
        } else {
            Err(format!("refusing to retrieve {uri}: only the compiled-in plan schema is available").into())
        }
    }
}

#[allow(clippy::expect_used)]
static PROPOSAL_VALIDATOR: LazyLock<Validator> = LazyLock::new(|| {
    let proposal: Value = serde_json::from_str(PROPOSAL_SCHEMA).expect("proposal schema is JSON");
    jsonschema::options().with_retriever(CompiledIn).build(&proposal).expect("proposal schema compiles")
});

/// One place a document breaks the schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "plan/")]
pub struct SchemaViolation {
    /// JSON Pointer to the offending value, `""` for the whole document.
    pub at: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error, ts_rs::TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, export_to = "plan/")]
pub enum PlanError {
    #[error("the plan is {size} bytes; the limit is {limit}")]
    TooLarge { size: usize, limit: usize },
    #[error("the plan is not valid JSON: {detail}")]
    Json { detail: String },
    #[error("the plan has no schema_version")]
    MissingVersion,
    #[error("schema version {found} is not supported; this KeyJutsu reads {supported}")]
    UnsupportedVersion { found: String, supported: String },
    #[error("the plan does not match the schema ({})", crate::count(violations.len(), "problem", "problems"))]
    Schema { violations: Vec<SchemaViolation> },
    /// The schema accepted it but the Rust model did not: the two disagree.
    #[error("internal error: the schema and the plan model disagree: {detail}")]
    ModelMismatch { detail: String },
    #[error("the plan's structure is invalid ({})", crate::count(problems.len(), "problem", "problems"))]
    Invalid { problems: Vec<Problem> },
}

/// A plan that passed every gate, with its graph.
#[derive(Debug, Clone)]
pub struct ValidPlan {
    plan: Plan,
    graph: PlanGraph,
}

impl ValidPlan {
    pub fn plan(&self) -> &Plan {
        &self.plan
    }

    pub fn graph(&self) -> &PlanGraph {
        &self.graph
    }

    pub fn into_plan(self) -> Plan {
        self.plan
    }

    /// Serialise deterministically: fields in model order, KeyJutsu state
    /// ordered by step id, optional fields omitted when absent.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(&self.plan).unwrap_or_default()
    }

    /// Re-check a plan that was edited in memory, through the same gates.
    pub fn revalidate(plan: Plan, proposal: bool) -> Result<Self, PlanError> {
        let value = serde_json::to_value(&plan).map_err(|e| PlanError::Json { detail: e.to_string() })?;
        check_value(value, proposal)
    }
}

/// Two valid plans are equal when their plans are; the graph follows from the plan.
impl PartialEq for ValidPlan {
    fn eq(&self, other: &Self) -> bool {
        self.plan == other.plan
    }
}

/// Parse a stored plan, which may carry KeyJutsu's own state.
pub fn parse_plan(text: &str) -> Result<ValidPlan, PlanError> {
    parse(text, false)
}

/// Parse what an agent returned. A proposal claiming readiness, proof, hashes
/// or approval is refused: those are only ever KeyJutsu's to record.
pub fn parse_proposal(text: &str) -> Result<ValidPlan, PlanError> {
    parse(text, true)
}

fn parse(text: &str, proposal: bool) -> Result<ValidPlan, PlanError> {
    if text.len() > MAX_PLAN_BYTES {
        return Err(PlanError::TooLarge { size: text.len(), limit: MAX_PLAN_BYTES });
    }
    let value: Value = serde_json::from_str(text).map_err(|e| PlanError::Json { detail: e.to_string() })?;
    check_value(value, proposal)
}

fn check_value(value: Value, proposal: bool) -> Result<ValidPlan, PlanError> {
    match value.get("schema_version") {
        None => return Err(PlanError::MissingVersion),
        Some(Value::String(v)) if v == SUPPORTED_VERSION => {}
        Some(other) => {
            let found = other.as_str().map(str::to_owned).unwrap_or_else(|| other.to_string());
            return Err(PlanError::UnsupportedVersion { found, supported: SUPPORTED_VERSION.into() });
        }
    }

    let validator: &Validator = if proposal { &PROPOSAL_VALIDATOR } else { &PLAN_VALIDATOR };
    let mut violations: Vec<SchemaViolation> = validator
        .iter_errors(&value)
        .map(|e| SchemaViolation { at: e.instance_path().to_string(), message: describe(&e) })
        .collect();
    if !violations.is_empty() {
        violations.sort_by(|a, b| (&a.at, &a.message).cmp(&(&b.at, &b.message)));
        violations.dedup();
        violations.truncate(50);
        return Err(PlanError::Schema { violations });
    }

    let plan: Plan =
        serde_json::from_value(value).map_err(|e| PlanError::ModelMismatch { detail: e.to_string() })?;
    let graph = analyse(&plan).map_err(|problems| PlanError::Invalid { problems })?;
    Ok(ValidPlan { plan, graph })
}

/// A violation message without the offending value in it. The validator's own
/// messages quote the value, which for a failure at the top of the document
/// means the whole plan: unreadable, and a copy of task content in every
/// error and, later, every log line.
fn describe(e: &jsonschema::ValidationError<'_>) -> String {
    let schema_path = e.schema_path().to_string();
    if schema_path.ends_with("/allOf/1/not") {
        return concat!(
            "a proposal may not contain the `keyjutsu` section: readiness, proof, hashes and approval ",
            "are recorded only by KeyJutsu"
        )
        .into();
    }
    let message = e.to_string();
    let value = e.instance().to_string();
    let message = if value.len() > 40 { message.replace(&value, "this value") } else { message };
    let mut short: String = message.chars().take(300).collect();
    if short.len() < message.len() {
        short.push('…');
    }
    short
}
