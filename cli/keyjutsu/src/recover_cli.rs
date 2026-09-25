//! `keyjutsu recover`: put back what a stopped run changed, when the
//! operator says so (§29).
//!
//! Without `--confirm` it only shows the recovery plan: nothing is rolled
//! back by default. With it, captured state is restored and approved recovery
//! commands run in a terminal, latest step first, each checked afterwards.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use keyjutsu_core::SessionOptions;
use keyjutsu_core::execute::Driver;
use keyjutsu_core::execution::PerformanceConfig;
use keyjutsu_core::plan::ApprovedSnapshot;
use keyjutsu_core::recovery::{RecoveryItem, RecoveryResult, plan_recovery, recover, recovery_dir};
use keyjutsu_core::terminal::{ProfileMode, ShellKind};

use crate::{console, run_cli};

pub struct RecoverArgs<'a> {
    pub snapshot: &'a Path,
    /// The checkpoint of the run to recover; the one next to the snapshot otherwise.
    pub from: Option<PathBuf>,
    pub steps: &'a [String],
    pub confirm: bool,
    pub clean: bool,
}

fn print_plan(items: &[RecoveryItem]) {
    for item in items {
        match item {
            RecoveryItem::Restore { step, what } => {
                println!("  {step}: restore what was captured before it ran");
                for w in what {
                    println!("      {w}");
                }
            }
            RecoveryItem::Commands { step, commands } => {
                println!("  {step}: run its approved recovery commands");
                for c in commands {
                    println!("      {c}");
                }
            }
            RecoveryItem::Cannot { step, reason } => println!("  {step}: cannot be recovered ({reason})"),
        }
    }
}

fn print_results(results: &[RecoveryResult]) -> bool {
    for r in results {
        println!("  {} {}", if r.recovered { "recovered" } else { "FAILED   " }, r.step);
        for c in &r.checks {
            let mark = match c.passed {
                Some(true) => "ok",
                Some(false) => "failed",
                None => "not checked",
            };
            println!("      {mark}: {} {}", c.check, c.detail);
        }
    }
    results.iter().all(|r| r.recovered)
}

pub fn run(args: RecoverArgs<'_>) -> ExitCode {
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
    let store = match keyjutsu_core::store::Store::open(&keyjutsu_core::store::default_root()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "keyjutsu: the encrypted store, which holds this account's approvals, cannot be opened: {e}"
            );
            return ExitCode::FAILURE;
        }
    };
    if let Err(reason) = keyjutsu_core::approvals::check_approval(&store, &snapshot) {
        eprintln!("keyjutsu: {reason}");
        return ExitCode::FAILURE;
    }
    let cp_path = args.from.unwrap_or_else(|| run_cli::checkpoint_path(args.snapshot));
    let checkpoint = match keyjutsu_core::approvals::load_checkpoint(&store, &cp_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("keyjutsu: cannot read the checkpoint {}: {e}", cp_path.display());
            return ExitCode::FAILURE;
        }
    };
    let items = match plan_recovery(&snapshot, &checkpoint, args.steps) {
        Ok(items) => items,
        Err(e) => {
            eprintln!("keyjutsu: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!("Recovery plan, latest step first:");
    print_plan(&items);
    if !args.confirm {
        println!();
        println!("Nothing has been changed. To carry this out, run the same command with --confirm.");
        return ExitCode::SUCCESS;
    }

    let dir = recovery_dir(&cp_path);
    let needs_terminal = items.iter().any(|i| matches!(i, RecoveryItem::Commands { .. }));
    let results = if needs_terminal {
        let mut options = SessionOptions::new(ShellKind::Pwsh);
        if args.clean {
            options.profile = ProfileMode::Clean;
        }
        let out: Arc<Mutex<Vec<RecoveryResult>>> = Arc::new(Mutex::new(Vec::new()));
        let (snap, cp, found, done) = (snapshot.clone(), checkpoint.clone(), items.clone(), out.clone());
        let controller: console::Controller = Box::new(move |session, events| {
            let driver = Driver { session: &session, events: &events };
            let r = recover(Some(&driver), &snap, &cp, &dir, &found, &PerformanceConfig::default(), &|_| {});
            if let Ok(mut d) = done.lock() {
                *d = r;
            }
            // The terminal was only for the recovery commands.
            session.close();
        });
        if let Err(e) = console::run(options, None, Some(controller)) {
            eprintln!("keyjutsu: {e}");
            return ExitCode::FAILURE;
        }
        out.lock().map(|r| r.clone()).unwrap_or_default()
    } else {
        recover(None, &snapshot, &checkpoint, &dir, &items, &PerformanceConfig::default(), &|_| {})
    };
    println!();
    if print_results(&results) {
        println!("Recovered.");
        ExitCode::SUCCESS
    } else {
        println!("Recovery stopped at the first step that could not be recovered.");
        ExitCode::FAILURE
    }
}
