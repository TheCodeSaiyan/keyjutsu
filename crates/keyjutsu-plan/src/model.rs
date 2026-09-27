//! The plan, as Rust types.
//!
//! These mirror `schemas/plan/v1/plan.schema.json` field for field. The schema
//! is checked first (see `parse.rs`), so these types never see a document the
//! schema refused; `tests/fixtures.rs` checks the two agree on every fixture.
//!
//! Optional fields are left out when absent rather than written as `null`, so
//! anything KeyJutsu serialises still validates against its own schema.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type StepId = String;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "plan/")]
pub struct Plan {
    pub schema_version: String,
    pub plan_id: String,
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub title: Option<String>,
    pub target: Target,
    pub agent: Agent,
    #[serde(default)]
    pub environment_assumptions: Vec<Assumption>,
    #[serde(default)]
    pub requirements: Vec<ToolRequirement>,
    #[serde(default)]
    pub phases: Vec<Phase>,
    pub steps: Vec<Step>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub execution_preferences: Option<ExecutionPreferences>,
    /// Written only by KeyJutsu. Never present in an agent's proposal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub keyjutsu: Option<KeyJutsuState>,
}

impl Plan {
    pub fn step(&self, id: &str) -> Option<&Step> {
        self.steps.iter().find(|s| s.id == id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "plan/")]
pub enum TargetKind {
    LocalWindows,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "plan/")]
pub struct Target {
    pub id: String,
    pub kind: TargetKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "plan/")]
pub enum AgentName {
    Codex,
    ClaudeCode,
    Gemini,
    GithubCopilot,
    Cursor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "plan/")]
pub struct Agent {
    pub name: AgentName,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "plan/")]
pub struct Assumption {
    pub description: String,
    pub check: Condition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "plan/")]
pub struct ToolRequirement {
    pub name: String,
    pub executable: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "plan/")]
pub enum ShellName {
    Pwsh,
    WindowsPowershell,
    Cmd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "plan/")]
pub enum Architecture {
    X64,
    Arm64,
    X86,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "plan/")]
pub enum Encoding {
    #[serde(rename = "utf-8")]
    Utf8,
    #[serde(rename = "utf-16le")]
    Utf16Le,
    #[serde(rename = "oem")]
    Oem,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "plan/")]
pub struct ShellBinding {
    pub kind: ShellName,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub architecture: Option<Architecture>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub encoding: Option<Encoding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "plan/")]
pub struct Command {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub purpose: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "plan/")]
pub enum StepKind {
    Command,
    Manual,
    UserInput,
    Credential,
    Validation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "plan/")]
pub enum Privilege {
    Standard,
    Administrator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "plan/")]
pub enum RiskLevel {
    Low,
    Normal,
    High,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "plan/")]
pub struct ProposedRisk {
    pub level: RiskLevel,
    pub rationale: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "plan/")]
pub enum ReversibilityLevel {
    Full,
    Partial,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "plan/")]
pub struct Reversibility {
    pub level: ReversibilityLevel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "plan/")]
pub enum EffectKind {
    FileCreated,
    FileModified,
    FileDeleted,
    RegistryValueSet,
    RegistryValueDeleted,
    ServiceState,
    ServiceStartType,
    PackageInstalled,
    PackageRemoved,
    ProcessStarted,
    GitCommit,
    GitPush,
    RestartRequired,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "plan/")]
pub struct Effect {
    pub kind: EffectKind,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "plan/")]
pub enum ExecutionMode {
    Performance,
    Assisted,
    AutoPerformance,
    Direct,
    UserInput,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "plan/")]
pub struct Step {
    pub id: StepId,
    pub title: String,
    pub objective: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reason: Option<String>,
    pub kind: StepKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub target_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub shell: Option<ShellBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub working_directory: Option<String>,
    #[serde(default)]
    pub commands: Vec<Command>,
    #[serde(default)]
    pub tool_requirements: Vec<ToolRequirement>,
    #[serde(default)]
    pub depends_on: Vec<StepId>,
    #[serde(default)]
    pub preconditions: Vec<Condition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub privilege: Option<Privilege>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub proposed_risk: Option<ProposedRisk>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reversibility: Option<Reversibility>,
    #[serde(default)]
    pub expected_effects: Vec<Effect>,
    #[serde(default)]
    pub visible_validation: Vec<Command>,
    #[serde(default)]
    pub internal_validation: Vec<Check>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub recovery: Option<Recovery>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub network: Option<NetworkContract>,
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub execution_mode: Option<ExecutionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub credential: Option<CredentialRequest>,
}

/// What a credential step asks for. The secret itself never appears in a
/// plan: the operator types it into the shell's own masked prompt, and later
/// steps refer to it by `variable`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "plan/")]
pub struct CredentialRequest {
    pub variable: String,
    pub prompt: String,
    pub kind: CredentialKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub target_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "plan/")]
pub enum CredentialKind {
    Secret,
    UsernameAndPassword,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "plan/")]
pub enum OutcomeIs {
    Succeeded,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "plan/")]
pub enum ServiceState {
    Running,
    Stopped,
    Paused,
    Missing,
}

/// A fact's expected value: the schema allows a string, number or boolean.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(untagged)]
#[ts(export, export_to = "plan/")]
pub enum FactValue {
    Bool(bool),
    Number(f64),
    Text(String),
}

/// The closed vocabulary of conditions. There is no expression form; see
/// `docs/schemas/plan.md`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "plan/")]
pub enum Condition {
    All(Vec<Condition>),
    Any(Vec<Condition>),
    Not(Box<Condition>),
    StepOutcome { step: StepId, is: OutcomeIs },
    ExitCode { step: StepId, equals: i64 },
    Fact { name: String, equals: FactValue },
    ToolVersion { tool: String, satisfies: String },
    PathExists { path: String },
    ServiceState { name: String, state: ServiceState },
}

impl Condition {
    /// Every step this condition refers to, in the order written.
    pub fn referenced_steps(&self) -> Vec<&str> {
        let mut out = Vec::new();
        self.collect_steps(&mut out);
        out
    }

    fn collect_steps<'a>(&'a self, out: &mut Vec<&'a str>) {
        match self {
            Condition::All(cs) | Condition::Any(cs) => cs.iter().for_each(|c| c.collect_steps(out)),
            Condition::Not(c) => c.collect_steps(out),
            Condition::StepOutcome { step, .. } | Condition::ExitCode { step, .. } => out.push(step),
            _ => {}
        }
    }

    /// Every version constraint in this condition, for up-front syntax checks.
    pub fn version_constraints(&self) -> Vec<&str> {
        match self {
            Condition::All(cs) | Condition::Any(cs) => {
                cs.iter().flat_map(Condition::version_constraints).collect()
            }
            Condition::Not(c) => c.version_constraints(),
            Condition::ToolVersion { satisfies, .. } => vec![satisfies],
            _ => Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "plan/")]
pub struct PathCheck {
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "plan/")]
pub struct ServiceCheck {
    pub name: String,
    pub state: ServiceState,
}

/// An internal validation check. Same closed-vocabulary rule as conditions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "plan/")]
pub enum Check {
    ExitCode { equals: i64 },
    ServiceState(ServiceCheck),
    PathExists(PathCheck),
    FileSha256 { path: String, sha256: String },
    JsonValue { path: String, pointer: String, equals: Value },
    TcpPortOpen { host: String, port: u16 },
    HttpStatus { url: String, status: u16 },
    TimeoutSeconds(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "plan/")]
pub enum RecoveryStrategy {
    RestoreCapturedState,
    Commands,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "plan/")]
pub enum CaptureKind {
    File,
    RegistryValue,
    ServiceState,
    PackageVersion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "plan/")]
pub struct Capture {
    pub kind: CaptureKind,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "plan/")]
pub struct Recovery {
    pub strategy: RecoveryStrategy,
    #[serde(default)]
    pub capture: Vec<Capture>,
    #[serde(default)]
    pub commands: Vec<Command>,
    #[serde(default)]
    pub validation: Vec<Check>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "plan/")]
pub enum Protocol {
    Https,
    Http,
    Ssh,
    Smb,
    Git,
    Winrm,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "plan/")]
pub struct Destination {
    pub host: String,
    pub protocol: Protocol,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub port: Option<u16>,
    pub purpose: String,
    pub at_runtime: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "plan/")]
pub struct NetworkContract {
    pub destinations: Vec<Destination>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "plan/")]
pub struct Artifact {
    pub name: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub publisher: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "plan/")]
pub struct Edge {
    pub from: StepId,
    pub to: StepId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub when: Option<Condition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "plan/")]
pub enum Boundary {
    WindowsRestart,
    SignOut,
    ShellRestart,
    WslRestart,
    DockerRestart,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "plan/")]
pub struct Phase {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub title: Option<String>,
    pub steps: Vec<StepId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub boundary_after: Option<Boundary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "plan/")]
pub enum SubmitPreference {
    AnyKey,
    RequireEnter,
    AutoSubmit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "plan/")]
pub enum ValidationDisplay {
    Meaningful,
    All,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "plan/")]
pub struct ExecutionPreferences {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub default_mode: Option<ExecutionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub submit: Option<SubmitPreference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub show_validation: Option<ValidationDisplay>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[ts(export, export_to = "plan/")]
pub enum Readiness {
    Ready,
    Blocked,
    Invalid,
    NeedsReview,
    RevalidationRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[ts(export, export_to = "plan/")]
pub enum ProofLevel {
    None,
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "plan/")]
pub enum EvidenceResult {
    Passed,
    Failed,
    NotApplicable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "plan/")]
pub struct Evidence {
    pub check: String,
    pub result: EvidenceResult,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "plan/")]
pub struct StepState {
    pub readiness: Readiness,
    pub proof_level: ProofLevel,
    #[serde(default)]
    pub remaining_uncertainty: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub assessed_risk: Option<RiskLevel>,
    #[serde(default)]
    pub risk_reasons: Vec<String>,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub step_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub approved: Option<bool>,
}

/// KeyJutsu's own record about a plan. Ordered by step id so serialising it
/// is deterministic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "plan/")]
pub struct KeyJutsuState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub revision: Option<u32>,
    pub steps: BTreeMap<StepId, StepState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub snapshot_hash: Option<String>,
    /// Who did what to which step, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provenance: Vec<ProvenanceEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "plan/")]
pub enum ActorKind {
    Agent,
    Operator,
    Keyjutsu,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "plan/")]
pub struct Actor {
    pub kind: ActorKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub agent: Option<Agent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "plan/")]
pub enum ProvenanceAction {
    Authored,
    Challenged,
    Revised,
    Edited,
    Validated,
    Approved,
    /// The operator read a reviewer's concern and decided it needs no change.
    Dismissed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "plan/")]
pub struct ProvenanceEvent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub step: Option<StepId>,
    pub actor: Actor,
    pub action: ProvenanceAction,
    pub at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub note: Option<String>,
}
