//! `keyjutsu plan hash | approve | verify | diff`: approval from the command
//! line. Running `approve` is the operator's explicit act; nothing here
//! approves anything on its own.

use std::path::Path;
use std::process::ExitCode;

use keyjutsu_core::fingerprint;
use keyjutsu_core::plan::approval::{ApprovalError, confirmation_phrase, is_critical};
use keyjutsu_core::plan::hash::affected_by_drift;
use keyjutsu_core::plan::{
    ApprovalBook, ApprovedSnapshot, ValidPlan, diff as plan_diff, parse_plan, seal, step_hashes,
};

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

pub fn approve(file: &Path, out: &Path, confirmations: &[String], force: bool) -> ExitCode {
    if out.exists() && !force {
        eprintln!("keyjutsu: {} already exists; pass --force to replace it", out.display());
        return ExitCode::from(2);
    }
    let plan = match load_plan(file) {
        Ok(p) => p,
        Err(c) => return c,
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
