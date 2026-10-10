//! Validation: how much KeyJutsu can show about a plan before anything runs.
//!
//! For each step it gathers the strongest safe evidence available:
//! whether the shell exists and is the right version, whether every command
//! line parses, whether every command resolves and every parameter exists,
//! whether required tools are installed at the right versions, whether
//! preconditions hold, whether the step needs rights KeyJutsu cannot yet get,
//! how risky it is by KeyJutsu's own rules, and, for the narrow set of
//! commands where it is trustworthy, what a `-WhatIf` dry run says it would
//! do. The result per step is a readiness, a proof level, the uncertainty that
//! remains, and the evidence behind each.
//!
//! What validation never does: run a program a plan names. Command lines are
//! parsed, not run; applications' versions are read from their files, not by
//! running `--version`. The single exception is `-WhatIf`, and only for
//! built-in management cmdlets whose arguments contain no expressions (see
//! [`powershell::what_if_blocker`]).

pub mod judge;
pub mod nested;
pub mod network;
pub mod paths;
pub mod powershell;
pub mod probe;
pub mod process;
pub mod profile;
pub mod risk;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use keyjutsu_plan::ValidPlan;
use keyjutsu_plan::condition::{Facts, StepResult, Truth, evaluate};
use keyjutsu_plan::model::{
    Actor, ActorKind, Check, Condition, EffectKind, FactValue, KeyJutsuState, Plan, ProvenanceAction,
    ProvenanceEvent, Readiness, ServiceState, ShellName, StepState,
};
use keyjutsu_plan::version::{Constraint, Version};
use keyjutsu_terminal::{ShellKind, shell};
use serde::Serialize;

use crate::judge::{Gathered, ToolFinding, judge};
use crate::powershell::{Analysis, analyse, what_if, what_if_blocker};

#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    /// Run `-WhatIf` dry runs where they are trustworthy.
    pub dry_run: bool,
    /// Whether the elevation broker can run Administrator steps.
    pub broker_available: bool,
    /// The work profile the plan is checked under, if any.
    pub profile: Option<profile::Profile>,
}

impl Default for Options {
    fn default() -> Self {
        Self { dry_run: true, broker_available: false, profile: None }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "validation/")]
pub struct AssumptionResult {
    pub description: String,
    /// `None` when it could not be decided.
    pub holds: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "validation/")]
pub struct Report {
    #[ts(type = "Record<string, import(\"../plan/StepState\").StepState>")]
    pub steps: BTreeMap<String, StepState>,
    pub assumptions: Vec<AssumptionResult>,
    /// Anything that stopped validation itself, such as a shell that could
    /// not be started to analyse the plan.
    pub problems: Vec<String>,
}

impl Report {
    /// Steps that are not READY, in execution order.
    pub fn not_ready(&self, plan: &ValidPlan) -> Vec<(String, Readiness)> {
        plan.graph()
            .topological_order()
            .filter_map(|id| {
                let r = self.steps.get(id).map(|s| s.readiness).unwrap_or(Readiness::Blocked);
                (r != Readiness::Ready).then(|| (id.to_owned(), r))
            })
            .collect()
    }

    /// The plan with this report recorded as KeyJutsu's own state, keeping
    /// its provenance and adding that KeyJutsu validated it at `at`.
    pub fn record_in(&self, plan: &Plan, at: &str) -> Plan {
        let mut out = plan.clone();
        let revision = plan.keyjutsu.as_ref().and_then(|k| k.revision).unwrap_or(0) + 1;
        let mut provenance = plan.keyjutsu.as_ref().map(|k| k.provenance.clone()).unwrap_or_default();
        provenance.push(ProvenanceEvent {
            step: None,
            actor: Actor { kind: ActorKind::Keyjutsu, agent: None },
            action: ProvenanceAction::Validated,
            at: at.to_owned(),
            note: None,
        });
        out.keyjutsu = Some(KeyJutsuState {
            revision: Some(revision),
            steps: self.steps.clone(),
            snapshot_hash: None,
            provenance,
        });
        out
    }
}

fn shell_kind(name: ShellName) -> ShellKind {
    match name {
        ShellName::Pwsh => ShellKind::Pwsh,
        ShellName::WindowsPowershell => ShellKind::WindowsPowershell,
        ShellName::Cmd => ShellKind::Cmd,
    }
}

/// What validation learned about the machine, as condition facts.
struct Observed<'a> {
    analysis: Option<&'a Analysis>,
}

impl Facts for Observed<'_> {
    fn step_result(&self, _: &str) -> Option<StepResult> {
        None // Nothing has run yet.
    }
    fn fact(&self, _: &str) -> Option<FactValue> {
        None // Named facts need collectors; none exist yet.
    }
    fn tool_version(&self, tool: &str) -> Option<String> {
        self.analysis?.tools.get(tool)?.as_ref()?.file_version.clone()
    }
    fn path_exists(&self, path: &str) -> Option<bool> {
        Some(Path::new(path).exists())
    }
    fn service_state(&self, name: &str) -> Option<ServiceState> {
        match self.analysis?.services.get(name)?.as_str() {
            "running" => Some(ServiceState::Running),
            "stopped" => Some(ServiceState::Stopped),
            "paused" => Some(ServiceState::Paused),
            "missing" => Some(ServiceState::Missing),
            _ => None,
        }
    }
}

fn collect_services(c: &Condition, out: &mut BTreeSet<String>) {
    match c {
        Condition::All(cs) | Condition::Any(cs) => cs.iter().for_each(|c| collect_services(c, out)),
        Condition::Not(c) => collect_services(c, out),
        Condition::ServiceState { name, .. } => {
            out.insert(name.clone());
        }
        _ => {}
    }
}

fn collect_tools(c: &Condition, out: &mut BTreeSet<String>) {
    match c {
        Condition::All(cs) | Condition::Any(cs) => cs.iter().for_each(|c| collect_tools(c, out)),
        Condition::Not(c) => collect_tools(c, out),
        Condition::ToolVersion { tool, .. } => {
            out.insert(tool.clone());
        }
        _ => {}
    }
}

fn describe(c: &Condition) -> String {
    serde_json::to_string(c).unwrap_or_default()
}

/// A path as Windows compares it: back slashes, no trailing one, any case.
fn normal(path: &str) -> String {
    path.replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

/// The earlier step, of those in `earlier`, that declares it creates or
/// changes `path`, or a file inside it: after that step, `path` exists,
/// though it may not yet. Validation looks at the machine as it is now; a
/// plan that makes a file and then uses it is not wrong for that.
fn provided_by<'a>(plan: &'a keyjutsu_plan::Plan, earlier: &BTreeSet<&str>, path: &str) -> Option<&'a str> {
    let want = normal(path);
    plan.steps
        .iter()
        .filter(|s| earlier.contains(s.id.as_str()))
        .find(|s| {
            s.expected_effects.iter().any(|e| {
                matches!(e.kind, EffectKind::FileCreated | EffectKind::FileModified) && {
                    let made = normal(&e.target);
                    made == want || made.starts_with(&format!("{want}\\"))
                }
            })
        })
        .map(|s| s.id.as_str())
}

/// Validate every step of `plan` against this machine.
pub fn validate(plan: &ValidPlan, options: Options) -> Report {
    let p = plan.plan();
    let mut problems = Vec::new();

    // Everything to ask the analysis shells about, in one pass per edition.
    // Tools named anywhere: requirements, and tool-version conditions in
    // assumptions, preconditions and branch conditions.
    let mut tools: BTreeSet<String> = p
        .requirements
        .iter()
        .chain(p.steps.iter().flat_map(|s| s.tool_requirements.iter()))
        .map(|r| r.executable.clone())
        .collect();
    for a in &p.environment_assumptions {
        collect_tools(&a.check, &mut tools);
    }
    for s in &p.steps {
        s.preconditions.iter().for_each(|c| collect_tools(c, &mut tools));
    }
    for e in &p.edges {
        if let Some(w) = &e.when {
            collect_tools(w, &mut tools);
        }
    }
    let mut services = BTreeSet::new();
    for a in &p.environment_assumptions {
        collect_services(&a.check, &mut services);
    }
    for s in &p.steps {
        s.preconditions.iter().for_each(|c| collect_services(c, &mut services));
        for check in &s.internal_validation {
            if let Check::ServiceState(sc) = check {
                services.insert(sc.name.clone());
            }
        }
    }

    let mut lines_by_edition: BTreeMap<&'static str, Vec<(String, &str)>> = BTreeMap::new();
    for s in &p.steps {
        let Some(sh) = &s.shell else { continue };
        let edition = match sh.kind {
            ShellName::Pwsh => "pwsh",
            ShellName::WindowsPowershell => "windows_powershell",
            ShellName::Cmd => continue,
        };
        let entry = lines_by_edition.entry(edition).or_default();
        for (i, c) in s.commands.iter().enumerate() {
            entry.push((format!("{}#c{i}", s.id), c.text.as_str()));
        }
        for (i, c) in s.visible_validation.iter().enumerate() {
            entry.push((format!("{}#v{i}", s.id), c.text.as_str()));
        }
        if let Some(r) = &s.recovery {
            for (i, c) in r.commands.iter().enumerate() {
                entry.push((format!("{}#r{i}", s.id), c.text.as_str()));
            }
        }
    }
    // Tools and services are looked up in one shell even if no step uses
    // PowerShell, preferring PowerShell 7.
    if lines_by_edition.is_empty() && (!tools.is_empty() || !services.is_empty()) {
        let edition = if shell::locate(ShellKind::Pwsh).is_some() { "pwsh" } else { "windows_powershell" };
        lines_by_edition.insert(edition, Vec::new());
    }

    let mut analyses: BTreeMap<&'static str, Analysis> = BTreeMap::new();
    let mut programs: BTreeMap<&'static str, std::path::PathBuf> = BTreeMap::new();
    let tool_names: Vec<&str> = tools.iter().map(String::as_str).collect();
    let service_names: Vec<&str> = services.iter().map(String::as_str).collect();
    for (edition, lines) in &lines_by_edition {
        let kind = if *edition == "pwsh" { ShellKind::Pwsh } else { ShellKind::WindowsPowershell };
        let Some(program) = shell::locate(kind) else { continue };
        match analyse(&program, lines, &tool_names, &service_names) {
            Ok(a) => {
                analyses.insert(edition, a);
                programs.insert(edition, program);
            }
            Err(e) => problems.push(format!("{}: {e}", kind.display_name())),
        }
    }
    // The first analysis answers machine-wide questions: tools, services, elevation.
    let machine = analyses.get("pwsh").or_else(|| analyses.values().next());
    let facts = Observed { analysis: machine };
    let elevated = machine.is_some_and(|a| a.elevated);
    let cmd_version = p
        .steps
        .iter()
        .any(|s| s.shell.as_ref().is_some_and(|sh| sh.kind == ShellName::Cmd))
        .then(|| shell::locate(ShellKind::Cmd).and_then(|c| shell::detect_version(ShellKind::Cmd, &c)))
        .flatten();

    // Plan-wide assumptions: if one fails, every step rests on something false.
    let mut assumptions = Vec::new();
    let mut plan_false = Vec::new();
    let mut plan_unknown = Vec::new();
    for a in &p.environment_assumptions {
        let holds = match evaluate(&a.check, &facts) {
            Truth::True => Some(true),
            Truth::False => {
                plan_false.push(format!("environment assumption \"{}\"", a.description));
                Some(false)
            }
            Truth::Unknown(_) => {
                plan_unknown.push(format!("environment assumption \"{}\"", a.description));
                None
            }
        };
        assumptions.push(AssumptionResult { description: a.description.clone(), holds });
    }

    // What a work profile looks at: every command a step would run, nested
    // ones included, which can take more analysis.
    let mut seen = match &options.profile {
        Some(_) => profile_seen(p, &analyses, &programs, &mut problems),
        None => BTreeMap::new(),
    };

    let mut steps = BTreeMap::new();
    for s in &p.steps {
        let (analysis, program, version) = match s.shell.as_ref().map(|sh| sh.kind) {
            Some(ShellName::Pwsh) => {
                (analyses.get("pwsh"), programs.get("pwsh"), analyses.get("pwsh").map(|a| a.version.clone()))
            }
            Some(ShellName::WindowsPowershell) => (
                analyses.get("windows_powershell"),
                programs.get("windows_powershell"),
                analyses.get("windows_powershell").map(|a| a.version.clone()),
            ),
            Some(ShellName::Cmd) => (None, None, cmd_version.clone()),
            None => (None, None, None),
        };
        // A shell that exists but could not be analysed still exists.
        let version = version.or_else(|| {
            let kind = shell_kind(s.shell.as_ref()?.kind);
            let path = shell::locate(kind)?;
            Some(shell::detect_version(kind, &path).unwrap_or_else(|| "unknown".into()))
        });
        let line_of = |id: String| analysis.and_then(|a| a.line(&id));
        let lines: Vec<(&str, Option<&powershell::LineAnalysis>)> = s
            .commands
            .iter()
            .enumerate()
            .map(|(i, c)| (c.text.as_str(), line_of(format!("{}#c{i}", s.id))))
            .collect();
        let mut support: Vec<(&str, Option<&powershell::LineAnalysis>)> = s
            .visible_validation
            .iter()
            .enumerate()
            .map(|(i, c)| (c.text.as_str(), line_of(format!("{}#v{i}", s.id))))
            .collect();
        if let Some(r) = &s.recovery {
            support.extend(
                r.commands
                    .iter()
                    .enumerate()
                    .map(|(i, c)| (c.text.as_str(), line_of(format!("{}#r{i}", s.id)))),
            );
        }

        let shell_version_ok = s.shell.as_ref().and_then(|sh| sh.version.as_deref()).and_then(|constraint| {
            let v = Version::parse_lenient(version.as_deref()?)?;
            Some(Constraint::parse(constraint).ok()?.matches(&v))
        });

        let tool_findings: Vec<ToolFinding> = p
            .requirements
            .iter()
            .chain(s.tool_requirements.iter())
            .map(|r| {
                let info = machine.and_then(|a| a.tools.get(&r.executable)).and_then(Option::as_ref);
                let version = info.and_then(|i| i.file_version.clone());
                let satisfied = r.version.as_deref().and_then(|c| {
                    Some(Constraint::parse(c).ok()?.matches(&Version::parse_lenient(version.as_deref()?)?))
                });
                ToolFinding {
                    name: r.executable.clone(),
                    found: info.is_some(),
                    version,
                    constraint: r.version.clone(),
                    satisfied,
                }
            })
            .collect();

        // Every step that runs before this one, and what they will have made.
        let earlier: BTreeSet<&str> = plan
            .graph()
            .ids()
            .iter()
            .filter(|x| plan.graph().descendants(x).contains(&s.id))
            .map(String::as_str)
            .collect();
        let mut deferred = Vec::new();

        let mut pre_false = plan_false.clone();
        let mut pre_unknown = plan_unknown.clone();
        for c in &s.preconditions {
            match evaluate(c, &facts) {
                Truth::True => {}
                // Not there yet, but an earlier step makes it: decided just
                // before this step runs, when it must hold.
                Truth::False if matches!(c, Condition::PathExists { path } if provided_by(p, &earlier, path).is_some()) => {
                    if let Condition::PathExists { path } = c {
                        let by = provided_by(p, &earlier, path).unwrap_or_default();
                        deferred.push(format!(
                            "{path} does not exist yet; `{by}` creates it first, and it is checked again just before this step runs"
                        ));
                    }
                }
                Truth::False => pre_false.push(describe(c)),
                // A condition on an earlier step's outcome is decided at run time.
                Truth::Unknown(_) if !c.referenced_steps().is_empty() => {}
                Truth::Unknown(_) => pre_unknown.push(describe(c)),
            }
        }

        let mut risk = risk::assess(s, &lines);
        let profile = options.profile.as_ref().map(|profile| {
            let breaches = profile.apply(s, &seen.remove(&s.id).unwrap_or_default(), &mut risk);
            (profile.title().to_owned(), breaches)
        });
        let mut dry_runs = Vec::new();
        let mut dry_run_skipped = Vec::new();
        if options.dry_run && risk.level > keyjutsu_plan::model::RiskLevel::Low {
            for (text, line) in &lines {
                match (line, program) {
                    (Some(line), Some(program)) => match what_if_blocker(line) {
                        None => match what_if(program, line, text) {
                            Ok(w) => dry_runs.push((*text, w)),
                            Err(e) => problems.push(format!("dry run of {}: {e}", crate::judge::quote(text))),
                        },
                        Some(why) => dry_run_skipped.push((*text, why)),
                    },
                    _ => dry_run_skipped.push((*text, "only PowerShell cmdlets can be dry-run")),
                }
            }
        }

        // A dry run that fails for want of something an earlier step makes
        // shows nothing about this step; it is not a failure of it.
        let made_earlier: Vec<(String, &str)> = p
            .steps
            .iter()
            .filter(|e| earlier.contains(e.id.as_str()))
            .flat_map(|e| {
                e.expected_effects
                    .iter()
                    .filter(|x| matches!(x.kind, EffectKind::FileCreated | EffectKind::FileModified))
                    .map(move |x| (x.target.clone(), e.id.as_str()))
            })
            .collect();
        dry_runs.retain(|(text, w): &(&str, powershell::WhatIf)| {
            let errors = normal(&w.errors.join(" "));
            match made_earlier.iter().find(|(target, _)| errors.contains(&normal(target))) {
                Some((target, by)) => {
                    deferred.push(format!(
                        "{} could not be dry-run yet: it needs {target}, which `{by}` creates first",
                        crate::judge::quote(text)
                    ));
                    false
                }
                None => true,
            }
        });
        let working_directory_exists = s.working_directory.as_ref().map(|d| {
            Path::new(d).is_dir()
                || provided_by(p, &earlier, d).is_some_and(|by| {
                    deferred.push(format!(
                        "the working directory {d} does not exist yet; `{by}` creates it first"
                    ));
                    true
                })
        });

        let gathered = Gathered {
            shell_version: version,
            shell_version_ok,
            lines,
            support_lines: support,
            tools: tool_findings,
            working_directory_exists,
            deferred,
            preconditions_false: pre_false,
            preconditions_unknown: pre_unknown,
            elevated,
            broker_available: options.broker_available,
            dry_runs,
            dry_run_skipped,
            risk: Some(risk),
            profile,
        };
        steps.insert(s.id.clone(), judge(s, &gathered));
    }

    Report { steps, assumptions, problems }
}

/// How deep KeyJutsu unpacks code nested in other code, such as a `cmd /c`
/// running `pwsh -EncodedCommand` running `Invoke-Expression`. Anything
/// deeper is opaque.
const MAX_NESTING: usize = 4;

/// PowerShell code still to parse: the step, how it was reached, the code.
type Pending = Vec<(String, String, String)>;

/// For each step, every command it would run and the code it could not see
/// into, for a work profile to judge. Nested PowerShell is parsed in further
/// rounds of analysis, one shell start per round, and only when there is any.
fn profile_seen(
    p: &Plan,
    analyses: &BTreeMap<&'static str, Analysis>,
    programs: &BTreeMap<&'static str, std::path::PathBuf>,
    problems: &mut Vec<String>,
) -> BTreeMap<String, profile::Seen> {
    let mut seen: BTreeMap<String, profile::Seen> = BTreeMap::new();
    let mut pending: Pending = Vec::new();
    for s in &p.steps {
        let entry = seen.entry(s.id.clone()).or_default();
        let texts = s
            .commands
            .iter()
            .enumerate()
            .map(|(i, c)| (format!("{}#c{i}", s.id), &c.text))
            .chain(s.visible_validation.iter().enumerate().map(|(i, c)| (format!("{}#v{i}", s.id), &c.text)))
            .chain(s.recovery.iter().flat_map(|r| {
                r.commands.iter().enumerate().map(|(i, c)| (format!("{}#r{i}", s.id), &c.text))
            }));
        let edition = match s.shell.as_ref().map(|sh| sh.kind) {
            Some(ShellName::Pwsh) => Some("pwsh"),
            Some(ShellName::WindowsPowershell) => Some("windows_powershell"),
            _ => None,
        };
        for (id, text) in texts {
            match edition {
                None => {
                    let mut at = Found { entry: &mut *entry, pending: &mut pending, step: &s.id };
                    for (name, args) in nested::cmd_segments(text) {
                        at.cmd_use(&name, args, None, 0);
                    }
                }
                Some(edition) => match analyses.get(edition).and_then(|a| a.line(&id)) {
                    Some(line) => {
                        Found { entry: &mut *entry, pending: &mut pending, step: &s.id }.line(line, None, 0)
                    }
                    None => entry.opaque.push(format!("{} could not be parsed", judge::quote(text))),
                },
            }
        }
    }

    // Nested PowerShell is parsed by PowerShell 7 where it is installed.
    let shell = programs.get("pwsh").or_else(|| programs.values().next()).cloned();
    let mut depth = 1;
    while !pending.is_empty() {
        let batch = std::mem::take(&mut pending);
        let lines: Vec<(String, &str)> =
            batch.iter().enumerate().map(|(i, (_, _, code))| (format!("n{i}"), code.as_str())).collect();
        let analysis = match &shell {
            Some(shell) if depth <= MAX_NESTING => match analyse(shell, &lines, &[], &[]) {
                Ok(a) => Some(a),
                Err(e) => {
                    problems.push(format!("analysing nested code: {e}"));
                    None
                }
            },
            _ => None,
        };
        for (i, (step, via, code)) in batch.iter().enumerate() {
            let entry = seen.entry(step.clone()).or_default();
            match analysis.as_ref().and_then(|a| a.line(&format!("n{i}"))) {
                Some(line) => Found { entry, pending: &mut pending, step }.line(line, Some(via), depth),
                None if depth > MAX_NESTING => entry
                    .opaque
                    .push(format!("{} is nested more than {MAX_NESTING} deep, {via}", judge::quote(code))),
                None => entry.opaque.push(format!("{}, {via}, could not be parsed", judge::quote(code))),
            }
        }
        depth += 1;
    }
    seen
}

/// Where the commands of one step are being collected.
struct Found<'a> {
    entry: &'a mut profile::Seen,
    pending: &'a mut Pending,
    step: &'a str,
}

impl Found<'_> {
    fn line(&mut self, line: &powershell::LineAnalysis, via: Option<&str>, depth: usize) {
        for c in &line.commands {
            let Some(name) = &c.name else {
                self.entry.opaque.push(match via {
                    Some(via) => format!("a command's name is computed when it runs, {via}"),
                    None => "a command's name is computed when the line runs".to_owned(),
                });
                continue;
            };
            let (name, written_as) = match c.resolved.as_deref().filter(|r| !r.is_empty()) {
                Some(target) => (target, Some(name.as_str())),
                None => (name.as_str(), None),
            };
            self.add(name, written_as, c.arguments.clone(), via, depth, c.static_arguments, c.from_pipeline);
        }
    }

    fn cmd_use(&mut self, name: &str, arguments: Vec<String>, via: Option<&str>, depth: usize) {
        // cmd expands %VARIABLES% when the line runs.
        let static_arguments = !arguments.iter().any(|a| a.contains('%'));
        self.add(name, None, arguments, via, depth, static_arguments, false);
    }

    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        name: &str,
        written_as: Option<&str>,
        arguments: Vec<String>,
        via: Option<&str>,
        depth: usize,
        static_arguments: bool,
        from_pipeline: bool,
    ) {
        let identity = profile::identity(name);
        let inner = nested::inner(&identity, &arguments, static_arguments, from_pipeline);
        let here = format!(
            "inside `{}`{}",
            written_as.unwrap_or(name),
            via.map(|v| format!(", {v}")).unwrap_or_default()
        );
        self.entry.uses.push(profile::Use {
            name: identity,
            written_as: written_as.map(profile::identity),
            arguments,
            via: via.map(str::to_owned),
        });
        match inner {
            None => {}
            Some(nested::Inner::Opaque(why)) => self.entry.opaque.push(match via {
                Some(via) => format!("{why}, {via}"),
                None => why,
            }),
            Some(nested::Inner::PowerShell(code)) => self.pending.push((self.step.to_owned(), here, code)),
            Some(nested::Inner::Cmd(text)) if depth < MAX_NESTING => {
                for (name, args) in nested::cmd_segments(&text) {
                    self.cmd_use(&name, args, Some(&here), depth + 1);
                }
            }
            Some(nested::Inner::Program(file, args)) if depth < MAX_NESTING => {
                self.cmd_use(&file, args, Some(&here), depth + 1);
            }
            Some(_) => self.entry.opaque.push(format!("code is nested more than {MAX_NESTING} deep, {here}")),
        }
    }
}
