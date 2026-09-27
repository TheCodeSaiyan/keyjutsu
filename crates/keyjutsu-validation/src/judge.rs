//! Turning collected evidence into a step's readiness, proof level and
//! remaining uncertainty. Pure: everything it needs is gathered first.
//!
//! Readiness, most serious first:
//!
//! - **INVALID**: the step cannot work as written (a syntax error, a parameter
//!   the command does not have, an ambiguous abbreviation).
//! - **BLOCKED**: it could work, but not on this machine as it stands (a
//!   missing shell, command or tool, a version outside the range, a missing
//!   working directory, a precondition that does not hold, Administrator
//!   rights KeyJutsu cannot yet obtain).
//! - **NEEDS_REVIEW**: nothing is known to be wrong, but a person should look
//!   (the agent rated the risk lower than KeyJutsu does, a fact nobody has
//!   collected, a dry run that failed and may depend on earlier steps).
//! - **READY** otherwise.
//!
//! Proof level says how much of that was actually shown rather than assumed.
//! READY with LOW proof is a real answer: nothing is wrong that KeyJutsu could
//! see, and it could not see much.

use keyjutsu_plan::model::{
    Evidence, EvidenceResult, Privilege, ProofLevel, Readiness, RiskLevel, Step, StepKind, StepState,
};

use crate::powershell::{LineAnalysis, WhatIf};
use crate::risk::Assessment;

/// Everything gathered about one step.
#[derive(Debug, Clone, Default)]
pub struct Gathered<'a> {
    /// The shell's version, or `None` if it is not installed.
    pub shell_version: Option<String>,
    /// Whether the shell's version satisfies the step's constraint; `None`
    /// when there is no constraint.
    pub shell_version_ok: Option<bool>,
    /// Each command line of the step with its analysis. PowerShell lines have
    /// one; cmd lines do not.
    pub lines: Vec<(&'a str, Option<&'a LineAnalysis>)>,
    /// Visible-validation and recovery lines, checked for syntax only.
    pub support_lines: Vec<(&'a str, Option<&'a LineAnalysis>)>,
    /// Required tools: name, whether found, version if read, constraint result.
    pub tools: Vec<ToolFinding>,
    pub working_directory_exists: Option<bool>,
    /// What could not be decided yet because an earlier step makes it first:
    /// a precondition, a working directory, a dry run. Decided again just
    /// before the step runs.
    pub deferred: Vec<String>,
    /// Preconditions that were false, and facts preconditions needed but nobody had.
    pub preconditions_false: Vec<String>,
    pub preconditions_unknown: Vec<String>,
    pub elevated: bool,
    pub broker_available: bool,
    pub dry_runs: Vec<(&'a str, WhatIf)>,
    pub dry_run_skipped: Vec<(&'a str, &'static str)>,
    pub risk: Option<Assessment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolFinding {
    pub name: String,
    pub found: bool,
    pub version: Option<String>,
    pub constraint: Option<String>,
    pub satisfied: Option<bool>,
}

struct Verdict {
    readiness: Readiness,
    evidence: Vec<Evidence>,
    uncertainty: Vec<String>,
}

impl Verdict {
    fn worsen(&mut self, to: Readiness) {
        let rank = |r: Readiness| match r {
            Readiness::Ready => 0,
            Readiness::NeedsReview => 1,
            Readiness::RevalidationRequired => 2,
            Readiness::Blocked => 3,
            Readiness::Invalid => 4,
        };
        if rank(to) > rank(self.readiness) {
            self.readiness = to;
        }
    }

    fn record(&mut self, check: &str, result: EvidenceResult, detail: impl Into<String>) {
        let detail = detail.into();
        self.evidence.push(Evidence {
            check: check.into(),
            result,
            detail: (!detail.is_empty()).then_some(detail),
        });
    }

    fn pass(&mut self, check: &str, detail: impl Into<String>) {
        self.record(check, EvidenceResult::Passed, detail);
    }

    fn fail(&mut self, check: &str, readiness: Readiness, detail: impl Into<String>) {
        self.record(check, EvidenceResult::Failed, detail);
        self.worsen(readiness);
    }
}

pub fn judge(step: &Step, g: &Gathered<'_>) -> StepState {
    let mut v = Verdict { readiness: Readiness::Ready, evidence: Vec::new(), uncertainty: Vec::new() };
    let operator_step = matches!(step.kind, StepKind::Manual | StepKind::UserInput | StepKind::Credential);
    let is_cmd = step.shell.as_ref().is_some_and(|s| s.kind == keyjutsu_plan::model::ShellName::Cmd);

    // Shell.
    if step.shell.is_some() {
        match &g.shell_version {
            None => v.fail("shell", Readiness::Blocked, "the step's shell is not installed"),
            Some(ver) => {
                v.pass("shell", format!("found, version {ver}"));
                match g.shell_version_ok {
                    Some(false) => v.fail(
                        "shell version",
                        Readiness::Blocked,
                        format!(
                            "{ver} is outside {}",
                            step.shell
                                .as_ref()
                                .and_then(|s| s.version.as_deref())
                                .unwrap_or("the required range")
                        ),
                    ),
                    Some(true) => v.pass("shell version", format!("{ver} is in range")),
                    None => {}
                }
            }
        }
    }

    // Syntax, commands and parameters, for every line the step can run.
    let mut all_parsed = !is_cmd && !g.lines.is_empty();
    for (text, analysis) in g.lines.iter().chain(g.support_lines.iter()) {
        let Some(a) = analysis else { continue };
        if a.syntax_errors.is_empty() {
            v.pass("syntax", format!("`{text}` parses"));
        } else {
            all_parsed = false;
            let first = &a.syntax_errors[0];
            v.fail(
                "syntax",
                Readiness::Invalid,
                format!("`{text}`: {} (column {})", first.message, first.column),
            );
        }
        for c in &a.commands {
            let name = c.name.as_deref().unwrap_or("(computed name)");
            match c.kind.as_deref() {
                None => {
                    all_parsed = false;
                    v.fail(
                        "commands",
                        Readiness::Blocked,
                        format!("`{name}` was not found, checked without your PowerShell profile"),
                    );
                }
                Some(kind) => {
                    let whence = c.path.as_deref().or(c.module.as_deref()).unwrap_or("");
                    v.pass(
                        "commands",
                        format!(
                            "`{name}` is a {kind}{}",
                            if whence.is_empty() { String::new() } else { format!(" ({whence})") }
                        ),
                    );
                    if kind == "Application" {
                        v.uncertainty
                            .push(format!("`{name}` is an external program; its arguments are not checked"));
                    }
                }
            }
            for p in &c.unknown_parameters {
                all_parsed = false;
                v.fail("parameters", Readiness::Invalid, format!("`{name}` has no parameter -{p}"));
            }
            for p in &c.ambiguous_parameters {
                all_parsed = false;
                v.fail(
                    "parameters",
                    Readiness::Invalid,
                    format!("-{p} is ambiguous for `{name}`; spell it out"),
                );
            }
            if c.unknown_parameters.is_empty()
                && c.ambiguous_parameters.is_empty()
                && !c.parameters_used.is_empty()
            {
                v.pass(
                    "parameters",
                    format!(
                        "`{name}` accepts {}",
                        c.parameters_resolved.iter().map(|p| format!("-{p}")).collect::<Vec<_>>().join(" ")
                    ),
                );
            }
        }
    }
    if is_cmd && !g.lines.is_empty() {
        v.record("syntax", EvidenceResult::NotApplicable, "cmd.exe has no parser KeyJutsu can ask");
        v.uncertainty.push("cmd.exe syntax is not checked before the step runs".into());
    }
    // Recovery is prepared before the step runs, so it must be
    // something KeyJutsu can actually carry out.
    if let Some(r) = &step.recovery {
        use keyjutsu_plan::model::{CaptureKind, RecoveryStrategy};
        match r.strategy {
            RecoveryStrategy::RestoreCapturedState if r.capture.is_empty() => {
                v.fail("recovery", Readiness::Invalid, "restores captured state but captures nothing")
            }
            RecoveryStrategy::Commands if r.commands.is_empty() => {
                v.fail("recovery", Readiness::Invalid, "recovers by commands but lists none")
            }
            _ => {}
        }
        for c in &r.capture {
            match c.kind {
                CaptureKind::RegistryValue => {
                    let upper = c.target.to_ascii_uppercase();
                    let named = c.target.rsplit_once('\\').is_some_and(|(k, n)| !n.is_empty() && k.len() > 6);
                    if !(upper.starts_with("HKCU:\\") || upper.starts_with("HKLM:\\")) || !named {
                        v.fail(
                            "recovery",
                            Readiness::Invalid,
                            format!(
                                "`{}`: a registry capture is HKCU:\\Key\\Value or HKLM:\\Key\\Value",
                                c.target
                            ),
                        );
                    }
                }
                CaptureKind::File if crate::paths::plain_file_path(&c.target).is_err() => v.fail(
                    "recovery",
                    Readiness::Invalid,
                    crate::paths::plain_file_path(&c.target).err().unwrap_or_default(),
                ),
                CaptureKind::File if std::path::Path::new(&c.target).is_dir() => v.fail(
                    "recovery",
                    Readiness::Invalid,
                    format!("`{}` is a folder; only files can be captured", c.target),
                ),
                CaptureKind::PackageVersion => {
                    v.record(
                        "recovery",
                        EvidenceResult::NotApplicable,
                        format!("package {}: KeyJutsu cannot restore package versions yet", c.target),
                    );
                    v.worsen(Readiness::NeedsReview);
                }
                _ => {}
            }
        }
        if r.strategy == RecoveryStrategy::RestoreCapturedState && !r.capture.is_empty() {
            v.pass("recovery", format!("{} item(s) captured before the step runs", r.capture.len()));
        }
    }

    // Network: every host a line names must be declared, and declared
    // for use while the step runs, not only for staging.
    let declared = step.network.as_ref().map(|n| n.destinations.as_slice()).unwrap_or_default();
    for (text, _) in g.lines.iter().chain(g.support_lines.iter()) {
        for c in crate::network::contacts(text) {
            match declared.iter().find(|d| d.host.eq_ignore_ascii_case(&c.host)) {
                None => v.fail(
                    "network",
                    Readiness::NeedsReview,
                    format!("contacts {} ({}), which the step does not declare", c.host, c.protocol),
                ),
                Some(d) if !d.at_runtime => v.fail(
                    "network",
                    Readiness::NeedsReview,
                    format!("contacts {} while it runs, but declares it only for staging", c.host),
                ),
                Some(d) => v.pass("network", format!("contacts {}, declared: {}", c.host, d.purpose)),
            }
        }
    }

    // Artifacts: pinned by hash, so what is approved is what runs.
    for a in &step.artifacts {
        if is_cmd {
            v.fail(
                "artifact",
                Readiness::Invalid,
                format!("{}: artifacts are handed to PowerShell steps only", a.name),
            );
        }
        match &a.sha256 {
            Some(sha) => {
                v.pass("artifact", format!("{} pinned to sha256 {}…", a.name, &sha[..sha.len().min(12)]))
            }
            None => v.fail(
                "artifact",
                Readiness::NeedsReview,
                format!(
                    "{} is not pinned yet: download it with Stage downloads in the app (or `keyjutsu plan stage --pin`), which records its hash, then validate again",
                    a.name
                ),
            ),
        }
    }

    // Credentials: asked for only through a prompt that masks them.
    if step.kind == StepKind::Credential {
        match &step.credential {
            None => v.fail("credential", Readiness::Invalid, "the step does not say what it asks for"),
            Some(_) if is_cmd => v.fail(
                "credential",
                Readiness::Invalid,
                "cmd.exe has no masked prompt, so the secret would be shown as it is typed; use PowerShell",
            ),
            Some(r) => v.pass(
                "credential",
                format!(
                    "asked for by PowerShell's own masked prompt, held in ${} for this run only",
                    r.variable
                ),
            ),
        }
    }
    if !is_cmd && !operator_step && !g.lines.is_empty() {
        v.uncertainty.push(
            "Checked without your PowerShell profile: aliases and functions it defines were not considered"
                .into(),
        );
    }

    // Tools.
    for t in &g.tools {
        match (t.found, t.satisfied) {
            (false, _) => {
                v.fail("tools", Readiness::Blocked, format!("`{}` is not installed or not on PATH", t.name))
            }
            (true, Some(false)) => v.fail(
                "tools",
                Readiness::Blocked,
                format!(
                    "`{}` {} is outside {}",
                    t.name,
                    t.version.as_deref().unwrap_or("?"),
                    t.constraint.as_deref().unwrap_or("?")
                ),
            ),
            (true, None) if t.constraint.is_some() => {
                v.record(
                    "tools",
                    EvidenceResult::NotApplicable,
                    format!(
                        "`{}` found, but its file carries no version to check against {}",
                        t.name,
                        t.constraint.as_deref().unwrap_or("?")
                    ),
                );
                v.worsen(Readiness::NeedsReview);
                v.uncertainty.push(format!("`{}`'s version could not be read without running it", t.name));
            }
            _ => v.pass(
                "tools",
                format!(
                    "`{}` found{}",
                    t.name,
                    t.version.as_ref().map(|x| format!(", version {x}")).unwrap_or_default()
                ),
            ),
        }
    }

    // Working directory.
    let cmd = step.shell.as_ref().is_some_and(|s| s.kind == keyjutsu_plan::model::ShellName::Cmd);
    if let Some(why) =
        step.working_directory.as_deref().and_then(|d| crate::paths::working_directory_problem(d, cmd))
    {
        v.fail("working directory", Readiness::Invalid, why);
    }
    match g.working_directory_exists {
        Some(true) => v.pass("working directory", step.working_directory.clone().unwrap_or_default()),
        Some(false) => v.fail(
            "working directory",
            Readiness::Blocked,
            format!("{} does not exist", step.working_directory.as_deref().unwrap_or("?")),
        ),
        None => {}
    }

    // Preconditions.
    for d in &g.deferred {
        v.record("preconditions", EvidenceResult::NotApplicable, d.clone());
        v.uncertainty.push(format!("Decided just before it runs: {d}"));
    }
    for c in &g.preconditions_false {
        v.fail("preconditions", Readiness::Blocked, format!("does not hold: {c}"));
    }
    for c in &g.preconditions_unknown {
        v.record("preconditions", EvidenceResult::NotApplicable, format!("could not be decided: {c}"));
        v.worsen(Readiness::NeedsReview);
    }

    // Privilege.
    if step.privilege == Some(Privilege::Administrator) {
        if g.elevated {
            v.pass("privilege", "KeyJutsu is running as Administrator");
        } else if g.broker_available {
            v.pass("privilege", "the elevation broker will run this step");
        } else {
            v.fail(
                "privilege",
                Readiness::Blocked,
                "needs Administrator, and keyjutsu-broker.exe is not installed next to KeyJutsu to run it elevated",
            );
        }
    }

    // Risk.
    if let Some(risk) = &g.risk {
        let proposed = step.proposed_risk.as_ref().map(|p| p.level);
        // KeyJutsu's rating is the one that counts either way. An agent that
        // rated a step lower is worth a second look only where the rating
        // matters: something High or Critical it called less. Below that,
        // it is said and KeyJutsu's rating applies.
        if let Some(p) = proposed
            && p < risk.level
            && risk.level < RiskLevel::High
        {
            v.pass(
                "risk",
                format!(
                    "{:?}, KeyJutsu's rating, which applies (the agent said {p:?}): {}",
                    risk.level,
                    risk.reasons.join("; ")
                ),
            );
        } else if let Some(p) = proposed
            && p < risk.level
        {
            v.fail(
                "risk",
                Readiness::NeedsReview,
                format!(
                    "the agent rated this {p:?}; KeyJutsu rates it {:?}: {}",
                    risk.level,
                    risk.reasons.join("; ")
                ),
            );
        } else {
            v.pass(
                "risk",
                format!(
                    "{:?}{}",
                    risk.level,
                    if risk.reasons.is_empty() {
                        String::new()
                    } else {
                        format!(": {}", risk.reasons.join("; "))
                    }
                ),
            );
        }
    }

    // Dry runs.
    let mut dry_run_clean = !g.dry_runs.is_empty();
    for (text, w) in &g.dry_runs {
        if w.errors.is_empty() {
            let targets = if w.operations.is_empty() {
                "no operations reported".to_owned()
            } else {
                w.operations.join(" | ")
            };
            v.pass("dry run", format!("`{text}` -WhatIf: {targets}"));
        } else {
            dry_run_clean = false;
            v.record(
                "dry run",
                EvidenceResult::Failed,
                format!("`{text}` -WhatIf: {}", w.errors.join(" | ")),
            );
            v.worsen(Readiness::NeedsReview);
            v.uncertainty.push("The dry run failed; an earlier step may create what it needs".into());
        }
    }
    for (text, why) in &g.dry_run_skipped {
        v.record("dry run", EvidenceResult::NotApplicable, format!("`{text}` not dry-run: {why}"));
    }

    // How much was shown.
    let low_risk = g.risk.as_ref().is_some_and(|r| r.level == RiskLevel::Low);
    let fully_dry_run = dry_run_clean && g.dry_run_skipped.is_empty();
    let proof = if operator_step || g.lines.is_empty() {
        v.uncertainty.push("KeyJutsu cannot check what the operator does in this step".into());
        ProofLevel::None
    } else if is_cmd || !all_parsed {
        ProofLevel::Low
    } else if low_risk || fully_dry_run {
        ProofLevel::High
    } else {
        ProofLevel::Medium
    };
    let proven_without_running = low_risk || fully_dry_run;
    if !proven_without_running && !operator_step && !g.lines.is_empty() {
        v.uncertainty.push("What this step changes is only proven by running it".into());
    }
    v.uncertainty.dedup();

    StepState {
        readiness: v.readiness,
        proof_level: proof,
        remaining_uncertainty: v.uncertainty,
        assessed_risk: g.risk.as_ref().map(|r| r.level),
        risk_reasons: g.risk.as_ref().map(|r| r.reasons.clone()).unwrap_or_default(),
        evidence: v.evidence,
        step_hash: None,
        approved: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn step(kind: &str) -> Step {
        serde_json::from_value(json!({"id": "s", "title": "T", "objective": "O", "kind": kind,
            "shell": {"kind": "pwsh"}, "commands": [{"text": "x"}]}))
        .unwrap()
    }

    fn line(unknown: &[&str]) -> LineAnalysis {
        serde_json::from_value(json!({"id": "l", "single_command": true, "commands": [{
            "name": "Get-Thing", "type": "Cmdlet", "static_arguments": true, "supports_what_if": false,
            "parameters_used": ["X"], "unknown_parameters": unknown
        }]}))
        .unwrap()
    }

    #[test]
    fn the_most_serious_finding_decides_readiness() {
        let l = line(&["X"]);
        let g = Gathered {
            shell_version: Some("7.6.6".into()),
            lines: vec![("Get-Thing -X", Some(&l))],
            working_directory_exists: Some(false),
            preconditions_unknown: vec!["a fact".into()],
            ..Gathered::default()
        };
        // Invalid (unknown parameter) outranks blocked (no working directory),
        // which outranks review (an undecided fact).
        let s = judge(&step("command"), &g);
        assert_eq!(s.readiness, Readiness::Invalid);
        assert_eq!(s.proof_level, ProofLevel::Low);
    }

    /// KeyJutsu's rating applies either way; only where it matters does an
    /// agent's lower rating need a second look.
    #[test]
    fn an_agent_rating_below_keyjutsus_needs_review_only_at_high_or_critical() {
        let rated = |agent: &str, keyjutsu: RiskLevel| {
            let mut s = step("command");
            s.proposed_risk = serde_json::from_value(json!({"level": agent, "rationale": "r"})).unwrap();
            let g = Gathered {
                shell_version: Some("7".into()),
                risk: Some(Assessment { level: keyjutsu, reasons: vec!["why".into()] }),
                ..Gathered::default()
            };
            judge(&s, &g)
        };
        let normal = rated("low", RiskLevel::Normal);
        assert_eq!(normal.readiness, Readiness::Ready, "{normal:#?}");
        assert!(
            normal.evidence.iter().any(|e| e.check == "risk"
                && e.detail.as_deref().is_some_and(|d| d.contains("the agent said Low")))
        );
        assert_eq!(rated("low", RiskLevel::High).readiness, Readiness::NeedsReview);
        assert_eq!(rated("normal", RiskLevel::Critical).readiness, Readiness::NeedsReview);
        assert_eq!(rated("high", RiskLevel::High).readiness, Readiness::Ready);
    }

    fn credential_step(shell: &str) -> Step {
        serde_json::from_value(json!({"id": "s", "title": "T", "objective": "O", "kind": "credential",
            "shell": {"kind": shell},
            "credential": {"variable": "TOKEN", "prompt": "Token", "kind": "secret"}}))
        .unwrap()
    }

    fn with_recovery(recovery: serde_json::Value) -> Step {
        serde_json::from_value(json!({"id": "s", "title": "T", "objective": "O", "kind": "command",
            "shell": {"kind": "pwsh"}, "commands": [{"text": "x"}], "recovery": recovery}))
        .unwrap()
    }

    #[test]
    fn a_recovery_that_cannot_be_carried_out_is_not_ready() {
        let g = Gathered { shell_version: Some("7".into()), ..Gathered::default() };
        let r = |v| judge(&with_recovery(v), &g).readiness;
        assert_eq!(r(json!({"strategy": "restore_captured_state"})), Readiness::Invalid);
        assert_eq!(r(json!({"strategy": "commands"})), Readiness::Invalid);
        assert_eq!(
            r(
                json!({"strategy": "restore_captured_state", "capture": [{"kind": "registry_value", "target": "Software\\X\\Y"}]})
            ),
            Readiness::Invalid
        );
        assert_eq!(
            r(
                json!({"strategy": "restore_captured_state", "capture": [{"kind": "file", "target": std::env::temp_dir().display().to_string()}]})
            ),
            Readiness::Invalid
        );
        assert_eq!(
            r(
                json!({"strategy": "restore_captured_state", "capture": [{"kind": "package_version", "target": "git"}]})
            ),
            Readiness::NeedsReview
        );
        assert_eq!(
            r(
                json!({"strategy": "restore_captured_state", "capture": [{"kind": "registry_value", "target": "HKCU:\\Software\\X\\Y"}]})
            ),
            Readiness::Ready
        );
    }

    fn with(extra: serde_json::Value, command: &str) -> Step {
        let mut v = json!({"id": "s", "title": "T", "objective": "O", "kind": "command",
            "shell": {"kind": "pwsh"}, "commands": [{"text": command}]});
        for (k, val) in extra.as_object().unwrap() {
            v[k] = val.clone();
        }
        serde_json::from_value(v).unwrap()
    }

    fn judged(step: &Step) -> StepState {
        let text = step.commands[0].text.as_str();
        let g =
            Gathered { shell_version: Some("7".into()), lines: vec![(text, None)], ..Gathered::default() };
        judge(step, &g)
    }

    #[test]
    fn an_undeclared_destination_needs_review() {
        let get = "Invoke-WebRequest -Uri https://downloads.example.com/tool.zip -OutFile tool.zip";
        let s = judged(&with(json!({}), get));
        assert_eq!(s.readiness, Readiness::NeedsReview);
        assert!(
            s.evidence
                .iter()
                .any(|e| e.detail.as_deref().is_some_and(|d| d.contains("downloads.example.com")))
        );

        let declared = json!({"network": {"destinations": [
            {"host": "downloads.example.com", "protocol": "https", "purpose": "the installer", "at_runtime": true}]}});
        assert_eq!(judged(&with(declared, get)).readiness, Readiness::Ready);

        let staging_only = json!({"network": {"destinations": [
            {"host": "downloads.example.com", "protocol": "https", "purpose": "the installer", "at_runtime": false}]}});
        assert_eq!(judged(&with(staging_only, get)).readiness, Readiness::NeedsReview);
    }

    #[test]
    fn an_artifact_must_be_pinned() {
        let use_it = "Copy-Item -LiteralPath $KJ_ARTIFACTS['tool.zip'] -Destination .";
        let unpinned = json!({"artifacts": [{"name": "tool.zip", "source": "https://example.com/tool.zip"}]});
        assert_eq!(judged(&with(unpinned, use_it)).readiness, Readiness::NeedsReview);
        let pinned = json!({"artifacts": [{"name": "tool.zip", "source": "https://example.com/tool.zip",
            "sha256": "a".repeat(64)}]});
        assert_eq!(judged(&with(pinned, use_it)).readiness, Readiness::Ready);
    }

    #[test]
    fn a_credential_is_only_asked_for_where_it_is_masked() {
        let g = Gathered { shell_version: Some("7".into()), ..Gathered::default() };
        assert_eq!(judge(&credential_step("pwsh"), &g).readiness, Readiness::Ready);
        assert_eq!(judge(&credential_step("cmd"), &g).readiness, Readiness::Invalid);
        let mut unsaid = credential_step("pwsh");
        unsaid.credential = None;
        assert_eq!(judge(&unsaid, &g).readiness, Readiness::Invalid);
    }

    #[test]
    fn operator_steps_claim_no_proof() {
        let s = judge(&step("manual"), &Gathered { shell_version: Some("7".into()), ..Gathered::default() });
        assert_eq!(s.readiness, Readiness::Ready);
        assert_eq!(s.proof_level, ProofLevel::None);
        assert!(s.remaining_uncertainty.iter().any(|u| u.contains("operator")));
    }

    #[test]
    fn a_missing_shell_blocks_everything_else() {
        let s = judge(&step("command"), &Gathered::default());
        assert_eq!(s.readiness, Readiness::Blocked);
    }
}
