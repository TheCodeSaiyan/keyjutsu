//! `keyjutsu plan validate | hash | approve | verify | diff`: validation and
//! approval from the command line. Running `approve` is the operator's explicit act; nothing here
//! approves anything on its own.

use std::path::Path;
use std::process::ExitCode;

use keyjutsu_core::fingerprint;
use keyjutsu_core::plan::approval::{ApprovalError, confirmation_phrase, is_critical};
use keyjutsu_core::plan::hash::affected_by_drift;
use keyjutsu_core::plan::model::{EvidenceResult, Readiness};
use keyjutsu_core::plan::{
    ApprovalBook, ApprovedSnapshot, ValidPlan, diff as plan_diff, parse_plan, seal, step_hashes,
};
use keyjutsu_core::validation::{self, Options, Report};

fn read(path: &Path) -> Result<String, ExitCode> {
    std::fs::read_to_string(path).map_err(|e| {
        eprintln!("keyjutsu: cannot read {}: {e}", path.display());
        ExitCode::from(2)
    })
}

fn load_plan(path: &Path) -> Result<ValidPlan, ExitCode> {
    parse_plan(&read(path)?).map_err(|e| {
        eprintln!("{}: {e}", path.display());
        eprintln!("  run `keyjutsu plan check --stored {}` for details", path.display());
        ExitCode::FAILURE
    })
}

fn short(hash: &str) -> &str {
    &hash[..hash.len().min(16)]
}

fn readiness_label(r: Readiness) -> &'static str {
    match r {
        Readiness::Ready => "READY",
        Readiness::Blocked => "BLOCKED",
        Readiness::Invalid => "INVALID",
        Readiness::NeedsReview => "REVIEW",
        Readiness::RevalidationRequired => "REVALIDATION REQUIRED",
    }
}

fn print_report(plan: &ValidPlan, report: &Report) {
    for id in plan.graph().topological_order() {
        let Some(s) = report.steps.get(id) else { continue };
        let risk = s.assessed_risk.map(|r| format!("{r:?}")).unwrap_or_else(|| "?".into());
        println!(
            "  {:<10} {:<7} risk {:<8} {id}",
            readiness_label(s.readiness),
            format!("{:?}", s.proof_level),
            risk
        );
        for e in s.evidence.iter().filter(|e| e.result == EvidenceResult::Failed) {
            println!("      ✕ {}: {}", e.check, e.detail.as_deref().unwrap_or(""));
        }
        if s.readiness == Readiness::Ready {
            for e in s.evidence.iter().filter(|e| e.check == "dry run" && e.result == EvidenceResult::Passed)
            {
                println!("      ✓ {}", e.detail.as_deref().unwrap_or(""));
            }
        }
        for u in &s.remaining_uncertainty {
            println!("      ? {u}");
        }
    }
    for a in &report.assumptions {
        let state = match a.holds {
            Some(true) => "holds",
            Some(false) => "DOES NOT HOLD",
            None => "undecided",
        };
        println!("  assumption {state}: {}", a.description);
    }
    for p in &report.problems {
        println!("  problem: {p}");
    }
}

pub fn validate(file: &Path, dry_run: bool, json: bool) -> ExitCode {
    let plan = match load_plan(file) {
        Ok(p) => p,
        Err(c) => return c,
    };
    let report = validation::validate(
        &plan,
        Options { dry_run, broker_available: keyjutsu_broker::broker_path().is_some() },
    );
    if json {
        println!("{}", serde_json::to_string_pretty(&report).unwrap_or_default());
    } else {
        print_report(&plan, &report);
        let not_ready = report.not_ready(&plan);
        println!();
        if not_ready.is_empty() {
            println!("All {} steps are ready.", report.steps.len());
        } else {
            println!("{} of {} steps are not ready.", not_ready.len(), report.steps.len());
        }
    }
    if report.not_ready(&plan).is_empty() && report.problems.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

pub fn hash(file: &Path) -> ExitCode {
    let plan = match load_plan(file) {
        Ok(p) => p,
        Err(c) => return c,
    };
    let hashes = step_hashes(plan.plan(), plan.graph());
    for id in plan.graph().topological_order() {
        println!("{}  {id}", hashes[id]);
    }
    ExitCode::SUCCESS
}

pub fn approve(file: &Path, out: &Path, confirmations: &[String], force: bool, dry_run: bool) -> ExitCode {
    if out.exists() && !force {
        eprintln!("keyjutsu: {} already exists; pass --force to replace it", out.display());
        return ExitCode::from(2);
    }
    let draft = match load_plan(file) {
        Ok(p) => p,
        Err(c) => return c,
    };
    // Validate first: nothing is approved that this machine cannot run, and
    // KeyJutsu's own risk assessment decides which steps are critical.
    let report = validation::validate(
        &draft,
        Options { dry_run, broker_available: keyjutsu_broker::broker_path().is_some() },
    );
    let not_ready = report.not_ready(&draft);
    if !not_ready.is_empty() || !report.problems.is_empty() {
        eprintln!("Not sealed: validation found steps that are not ready.");
        eprintln!();
        print_report(&draft, &report);
        return ExitCode::FAILURE;
    }
    let plan = match ValidPlan::revalidate(report.record_in(draft.plan(), &fingerprint::now_rfc3339()), false)
    {
        Ok(p) => p,
        Err(e) => {
            eprintln!("keyjutsu: recording the validation failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    let at = fingerprint::now_rfc3339();
    let mut book = ApprovalBook::new();
    book.approve_all_except_critical(&plan, &at);

    for c in confirmations {
        let Some((step, phrase)) = c.split_once('=') else {
            eprintln!("keyjutsu: --confirm takes STEP=PHRASE, got `{c}`");
            return ExitCode::from(2);
        };
        match book.approve(&plan, step.trim(), &at, Some(phrase)) {
            Ok(()) => {}
            Err(ApprovalError::ConfirmationMismatch { step, phrase }) => {
                eprintln!("keyjutsu: the confirmation for `{step}` does not match. Type exactly: {phrase}");
                return ExitCode::FAILURE;
            }
            Err(e) => {
                eprintln!("keyjutsu: {e}");
                return ExitCode::FAILURE;
            }
        }
    }

    let fp = fingerprint::collect(Some(plan.plan()));
    let snapshot = match seal(&plan, &book, Some(fp), &at) {
        Ok(s) => s,
        Err(_) => {
            eprintln!("Not sealed: these critical steps need their own typed confirmation.");
            eprintln!();
            for id in plan.graph().topological_order() {
                let Some(step) = plan.plan().step(id) else { continue };
                if is_critical(plan.plan(), step)
                    && !confirmations.iter().any(|c| c.split('=').next() == Some(id))
                {
                    eprintln!("  CRITICAL ACTION  {id}: {}", step.title);
                    for command in &step.commands {
                        eprintln!("    runs:          {}", command.text);
                    }
                    for effect in &step.expected_effects {
                        eprintln!("    target:        {} ({:?})", effect.target, effect.kind);
                    }
                    if let Some(risk) = &step.proposed_risk {
                        eprintln!("    impact:        {}", risk.rationale);
                    }
                    if let Some(state) = plan.plan().keyjutsu.as_ref().and_then(|k| k.steps.get(id)) {
                        for reason in &state.risk_reasons {
                            eprintln!("    why critical:  {reason}");
                        }
                        for e in state.evidence.iter().filter(|e| e.check == "dry run") {
                            eprintln!("    dry run:       {}", e.detail.as_deref().unwrap_or(""));
                        }
                    }
                    match &step.reversibility {
                        Some(r) => eprintln!(
                            "    reversibility: {:?}{}",
                            r.level,
                            r.notes.as_deref().map(|n| format!(": {n}")).unwrap_or_default()
                        ),
                        None => eprintln!("    reversibility: not stated"),
                    }
                    let recovery = step.recovery.as_ref().map(|r| format!("{:?}", r.strategy));
                    eprintln!("    recovery:      {}", recovery.as_deref().unwrap_or("none stated"));
                    eprintln!("    confirm: --confirm {id}=\"{}\"", confirmation_phrase(step));
                    eprintln!();
                }
            }
            return ExitCode::FAILURE;
        }
    };
    // The record is what `keyjutsu run` trusts, not the file's own hashes.
    if let Err(e) = keyjutsu_core::store::Store::open(&keyjutsu_core::store::default_root())
        .and_then(|s| keyjutsu_core::approvals::record_approval(&s, &snapshot))
    {
        eprintln!("keyjutsu: the approval could not be recorded in the encrypted store: {e}");
        return ExitCode::FAILURE;
    }
    if let Err(e) = std::fs::write(out, snapshot.to_json()) {
        eprintln!("keyjutsu: cannot write {}: {e}", out.display());
        return ExitCode::FAILURE;
    }
    println!("Sealed {} steps into {}", snapshot.step_hashes().len(), out.display());
    println!("Snapshot {}", snapshot.snapshot_hash());
    ExitCode::SUCCESS
}

pub fn verify(file: &Path, environment: bool) -> ExitCode {
    let text = match read(file) {
        Ok(t) => t,
        Err(c) => return c,
    };
    let snapshot = match ApprovedSnapshot::from_json(&text) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}: {e}", file.display());
            return ExitCode::FAILURE;
        }
    };
    println!("Intact: {} steps approved, sealed {}", snapshot.step_hashes().len(), snapshot.sealed_at());
    println!("Snapshot {}", snapshot.snapshot_hash());
    if !environment {
        return ExitCode::SUCCESS;
    }
    let Some(then) = snapshot.fingerprint() else {
        println!("No environment was recorded when it was sealed, so drift cannot be checked.");
        return ExitCode::FAILURE;
    };
    let now = fingerprint::collect(Some(snapshot.plan()));
    let drifts = then.drift(&now);
    if drifts.is_empty() {
        println!("Environment unchanged since approval.");
        return ExitCode::SUCCESS;
    }
    println!();
    println!("The environment has changed since approval:");
    for d in &drifts {
        println!(
            "  {}: {} -> {}",
            d.what,
            d.before.as_deref().unwrap_or("absent"),
            d.after.as_deref().unwrap_or("absent")
        );
    }
    let affected = affected_by_drift(snapshot.plan(), snapshot.graph(), &drifts);
    if affected.is_empty() {
        println!("No step depends on what changed.");
        ExitCode::SUCCESS
    } else {
        println!("{} step(s) require revalidation: {}", affected.len(), affected.join(", "));
        ExitCode::FAILURE
    }
}

pub fn diff(old: &Path, new: &Path) -> ExitCode {
    let (a, b) = match (load_plan(old), load_plan(new)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(c), _) | (_, Err(c)) => return c,
    };
    let d = plan_diff(a.plan(), b.plan(), b.graph());
    if d.is_empty() {
        println!("No changes.");
        return ExitCode::SUCCESS;
    }
    for id in &d.added {
        println!("added    {id}");
    }
    for id in &d.removed {
        println!("removed  {id}");
    }
    for c in &d.changed {
        let note = if c.execution_relevant { "" } else { "  (wording only)" };
        println!("changed  {}: {}{note}", c.step, c.fields.join(", "));
    }
    for e in &d.edges_changed {
        println!("flow     {e}");
    }
    for f in &d.plan_fields {
        println!("plan     {f}");
    }
    let (ha, hb) = (step_hashes(a.plan(), a.graph()), step_hashes(b.plan(), b.graph()));
    println!();
    if d.affected.is_empty() {
        println!("No step needs revalidation.");
    } else {
        println!("{} step(s) require revalidation:", d.affected.len());
        for id in &d.affected {
            let was = ha.get(id).map(|h| short(h)).unwrap_or("(new)");
            println!("  {id}  {was} -> {}", short(&hb[id]));
        }
    }
    ExitCode::SUCCESS
}

/// `keyjutsu plan stage`: download, verify and keep each artifact (§30).
pub fn stage(file: &Path, pin_to: Option<&Path>) -> ExitCode {
    let plan = match load_plan(file) {
        Ok(p) => p,
        Err(c) => return c,
    };
    let store = keyjutsu_core::artifacts::default_store();
    let wanted = keyjutsu_core::artifacts::artifacts(plan.plan());
    if wanted.is_empty() {
        println!("This plan downloads nothing.");
        return ExitCode::SUCCESS;
    }
    let at = fingerprint::now_rfc3339();
    let mut staged = Vec::new();
    let mut failed = false;
    for a in wanted {
        match keyjutsu_core::artifacts::stage(&store, a, &at) {
            Ok(s) => {
                println!("  staged  {}  sha256 {}  {} bytes", s.name, s.sha256, s.size);
                println!("          from {}", s.source);
                if !s.was_pinned {
                    println!("          not pinned by the plan: review this hash before approving");
                }
                staged.push(s);
            }
            Err(e) => {
                println!("  FAILED  {}: {e}", a.name);
                failed = true;
            }
        }
    }
    println!("Kept in {}", store.display());
    if let Some(out) = pin_to {
        let pinned = keyjutsu_core::artifacts::pin(plan.plan(), &staged);
        match serde_json::to_string_pretty(&pinned)
            .map_err(|e| e.to_string())
            .and_then(|t| std::fs::write(out, t).map_err(|e| e.to_string()))
        {
            Ok(()) => println!("Wrote the pinned plan to {}. Validate and approve that one.", out.display()),
            Err(e) => {
                eprintln!("keyjutsu: cannot write {}: {e}", out.display());
                return ExitCode::FAILURE;
            }
        }
    }
    if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}
