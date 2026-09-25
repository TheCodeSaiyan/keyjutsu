//! `keyjutsu run`: execute an approved snapshot in this console.
//!
//! Before anything runs, the snapshot is re-checked (every hash), the
//! preflight must pass (validated, every step READY, one shell), and this
//! machine is compared with the one it was approved on: if anything a step
//! depends on has changed since, nothing runs (§33).
//!
//! During the run, nothing is printed into the terminal: the performance is
//! the shell's own output. Progress goes to the console title, and the
//! outcome is printed once the session ends.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use keyjutsu_core::SessionOptions;
use keyjutsu_core::execute::{
    Checkpoint, Driver, ExecuteOptions, ExecutionEvent, Outcome, execute, preflight,
};
use keyjutsu_core::execution::ExecutionMode;
use keyjutsu_core::fingerprint;
use keyjutsu_core::plan::ApprovedSnapshot;
use keyjutsu_core::plan::hash::affected_by_drift;
use keyjutsu_core::plan::model::ShellName;
use keyjutsu_core::terminal::{ProfileMode, ShellKind};

use crate::console;

pub struct RunArgs<'a> {
    pub snapshot: &'a Path,
    pub mode: Option<ExecutionMode>,
    pub clean: bool,
    /// The checkpoint to continue from, if any.
    pub resume: Option<PathBuf>,
    pub settle: &'a [String],
}

pub fn checkpoint_path(snapshot: &Path) -> PathBuf {
    snapshot.with_extension("checkpoint.json")
}

pub fn run(args: RunArgs<'_>) -> ExitCode {
    let text = match std::fs::read_to_string(args.snapshot) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("keyjutsu: cannot read {}: {e}", args.snapshot.display());
            return ExitCode::from(2);
        }
    };
    let snapshot = match ApprovedSnapshot::from_json(&text) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}: {e}", args.snapshot.display());
            return ExitCode::FAILURE;
        }
    };
    if let Err(reason) = preflight(&snapshot) {
        eprintln!("keyjutsu: will not run: {reason}");
        return ExitCode::FAILURE;
    }
    if let Some(then) = snapshot.fingerprint() {
        let now = fingerprint::collect(Some(snapshot.plan()));
        let drifts = then.drift(&now);
        let affected = affected_by_drift(snapshot.plan(), snapshot.graph(), &drifts);
        if !affected.is_empty() {
            eprintln!("keyjutsu: will not run: this machine has changed since the plan was approved.");
            for d in &drifts {
                eprintln!(
                    "  {}: {} -> {}",
                    d.what,
                    d.before.as_deref().unwrap_or("absent"),
                    d.after.as_deref().unwrap_or("absent")
                );
            }
            eprintln!("  steps that require revalidation: {}", affected.join(", "));
            return ExitCode::FAILURE;
        }
    }

    let mut settled = BTreeMap::new();
    for s in args.settle {
        match s.split_once('=') {
            Some((step, "succeeded")) => {
                settled.insert(step.to_owned(), true);
            }
            Some((step, "failed")) => {
                settled.insert(step.to_owned(), false);
            }
            _ => {
                eprintln!("keyjutsu: --settle takes STEP=succeeded or STEP=failed, got `{s}`");
                return ExitCode::from(2);
            }
        }
    }
    let cp_path = checkpoint_path(args.snapshot);
    let resume = match &args.resume {
        Some(from) => match Checkpoint::load(from) {
            Ok(c) => Some(c),
            Err(e) => {
                eprintln!("keyjutsu: cannot resume from {}: {e}", from.display());
                return ExitCode::FAILURE;
            }
        },
        None => None,
    };

    let shell = match snapshot.plan().steps.iter().find_map(|s| s.shell.as_ref().map(|sh| sh.kind)) {
        Some(ShellName::WindowsPowershell) => ShellKind::WindowsPowershell,
        Some(ShellName::Cmd) => ShellKind::Cmd,
        _ => ShellKind::Pwsh,
    };
    let mut options = SessionOptions::new(shell);
    if args.clean {
        options.profile = ProfileMode::Clean;
    }

    let result: Arc<Mutex<Option<(Outcome, Checkpoint)>>> = Arc::new(Mutex::new(None));
    let (snap, out, mode) = (snapshot.clone(), result.clone(), args.mode);
    let exec_options =
        ExecuteOptions { mode, checkpoint: Some(cp_path.clone()), settled, ..ExecuteOptions::default() };
    let controller: console::Controller = Box::new(move |session, events| {
        let title = |t: &str| {
            let _ = crossterm::execute!(std::io::stdout(), crossterm::terminal::SetTitle(t));
        };
        let observe = |e: ExecutionEvent| match &e {
            ExecutionEvent::StepStarting { title: t, .. } => title(t),
            // Staged typing is off: the operator must stop mashing and answer
            // for real, in the shell's own masked prompt.
            ExecutionEvent::CredentialRequired { prompt, .. } => {
                title(&format!("Credential required: {prompt}. Stop typing, then press Enter."));
            }
            _ => {}
        };
        let done = execute(
            &Driver { session: &session, events: &events },
            &snap,
            resume,
            &exec_options,
            &fingerprint::now_rfc3339,
            &observe,
        );
        if let Ok(mut r) = out.lock() {
            *r = Some(done);
        }
    });

    if let Err(e) = console::run(options, None, Some(controller)) {
        eprintln!("keyjutsu: {e}");
        return ExitCode::FAILURE;
    }
    let Some((outcome, checkpoint)) = result.lock().ok().and_then(|mut r| r.take()) else {
        println!("The shell ended before the plan finished. Checkpoint: {}", cp_path.display());
        return ExitCode::FAILURE;
    };
    for run in &checkpoint.runs {
        let mark = if run.succeeded { "ok    " } else { "FAILED" };
        println!("  {mark} {}", run.step);
        for c in run.checks.iter().filter(|c| c.passed != Some(true)) {
            println!("         {}: {}", c.check, c.detail);
        }
    }
    match outcome {
        Outcome::Complete => {
            println!("Complete: {} steps.", checkpoint.runs.len());
            ExitCode::SUCCESS
        }
        Outcome::Failed { step, expected, actual } => {
            println!();
            println!("Step `{step}` failed. The plan stopped there.");
            println!("  expected: {expected}");
            println!("  actual:   {actual}");
            println!();
            println!("Next: `keyjutsu plan revise` with your guidance, validate and approve again, then");
            println!("`keyjutsu run <new snapshot> --resume {}`;", cp_path.display());
            println!("steps that already succeeded and are unchanged are not run again.");
            ExitCode::FAILURE
        }
        Outcome::Aborted { step, in_doubt } => {
            println!("Disarmed{}.", step.map(|s| format!(" during `{s}`")).unwrap_or_default());
            if in_doubt {
                println!(
                    "A command had been submitted, so its effect is unknown. Check, then resume with --settle."
                );
            }
            ExitCode::FAILURE
        }
        Outcome::Blocked { reason } => {
            println!("Could not continue: {reason}");
            ExitCode::FAILURE
        }
    }
}
