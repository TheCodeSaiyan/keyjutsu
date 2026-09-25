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
use keyjutsu_core::git;

/// Where the plan runs, relative to the operator's working tree (§31).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Isolation {
    /// A temporary worktree on a new local branch; the working tree is untouched.
    Worktree,
    /// A new local branch, switched to in place.
    Branch,
}

pub struct RunArgs<'a> {
    pub snapshot: &'a Path,
    pub mode: Option<ExecutionMode>,
    pub clean: bool,
    /// The checkpoint to continue from, if any.
    pub resume: Option<PathBuf>,
    pub settle: &'a [String],
    pub isolate: Option<Isolation>,
    /// Keep no record of the session (§35).
    pub ephemeral: bool,
}

/// Where a run keeps its Git record: next to the snapshot.
pub fn git_folder(snapshot: &Path) -> PathBuf {
    snapshot.with_extension("git")
}

fn print_git(r: &git::RepoReport, snapshot: &Path) {
    if r.keyjutsu.is_empty() && r.untouched.is_empty() {
        return;
    }
    println!();
    println!("Git: {}{}", r.root, r.branch.as_ref().map(|b| format!(" ({b})")).unwrap_or_default());
    if r.keyjutsu.is_empty() {
        println!("  KeyJutsu changed nothing here.");
    } else {
        println!("  Changed by KeyJutsu:");
        for k in &r.keyjutsu {
            let note = if k.was_already_changed {
                "  (you had changed it too; your part is not counted)"
            } else {
                ""
            };
            println!("    {} {}{note}", k.status, k.path);
        }
    }
    if !r.untouched.is_empty() {
        println!("  Your own changes, untouched: {}", r.untouched.join(", "));
    }
    if let Some((from, to)) = &r.head_moved {
        println!("  HEAD moved from {} to {}.", &from[..from.len().min(10)], &to[..to.len().min(10)]);
    }
    println!("  KeyJutsu's diff: keyjutsu git diff {}", snapshot.display());
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

    // Resuming past a session boundary needs the operator's word, typed
    // here on the plain console before anything starts (§32).
    let mut resume_gate = None;
    if let Some(wait) = resume.as_ref().and_then(|c| c.boundary.as_ref()) {
        let what = keyjutsu_core::boundary::describe(wait.kind);
        println!("This plan stopped after phase `{}` for a {what}.", wait.after_phase);
        println!(
            "Before continuing, KeyJutsu checks that the {what} happened, compares this machine with the"
        );
        println!("one the plan was approved on, and checks again what the earlier phases achieved.");
        print!("Type RESUME to check and continue: ");
        let _ = std::io::Write::flush(&mut std::io::stdout());
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        if line.trim() != "RESUME" {
            println!("Not resumed. Nothing ran.");
            return ExitCode::FAILURE;
        }
        let gate: keyjutsu_core::boundary::ResumeGate = Arc::new(|_| true);
        resume_gate = Some(gate);
    }

    // Critical steps approved more than an hour ago are confirmed again,
    // here on the plain console, before anything starts (§28).
    if keyjutsu_core::execute::needs_reconfirmation(snapshot.sealed_at(), fingerprint::now_secs()) {
        let plan = snapshot.plan();
        for id in snapshot.graph().topological_order() {
            let Some(step) = plan.step(id) else { continue };
            if !keyjutsu_core::plan::approval::is_critical(plan, step) {
                continue;
            }
            let c = keyjutsu_core::execute::critical_confirmation(plan, step);
            println!();
            println!("CRITICAL ACTION  {}  (approved {})", c.title, snapshot.sealed_at());
            for command in &c.commands {
                println!("  runs:      {command}");
            }
            for target in &c.targets {
                println!("  target:    {target}");
            }
            for i in &c.impact {
                println!("  impact:    {i}");
            }
            println!("  recovery:  {}", c.recovery);
            print!("Type {} to let it run: ", c.phrase);
            let _ = std::io::Write::flush(&mut std::io::stdout());
            let mut line = String::new();
            let _ = std::io::stdin().read_line(&mut line);
            if line.trim() != c.phrase {
                println!("Not confirmed. Nothing ran.");
                return ExitCode::FAILURE;
            }
        }
    }

    let shell = match snapshot.plan().steps.iter().find_map(|s| s.shell.as_ref().map(|sh| sh.kind)) {
        Some(ShellName::WindowsPowershell) => ShellKind::WindowsPowershell,
        Some(ShellName::Cmd) => ShellKind::Cmd,
        _ => ShellKind::Pwsh,
    };
    let started = fingerprint::now_rfc3339();
    let mut options = SessionOptions::new(shell);
    if args.clean {
        options.profile = ProfileMode::Clean;
    }

    // Git (§31): isolate if asked, then record every repository the plan
    // works in, so its changes can be told from the operator's afterwards.
    let mut start = std::env::current_dir().unwrap_or_default();
    if let Some(isolation) = args.isolate {
        let Some(root) = git::find_root(&start) else {
            eprintln!("keyjutsu: --isolate needs a Git repository, and {} is not in one", start.display());
            return ExitCode::FAILURE;
        };
        let branch = git::isolation_branch(&snapshot.plan().plan_id, snapshot.snapshot_hash());
        match isolation {
            Isolation::Worktree => {
                let inside = git::steps_inside(&root, snapshot.plan());
                if !inside.is_empty() {
                    eprintln!(
                        "keyjutsu: steps {} name folders in {}, which a worktree elsewhere would not isolate",
                        inside.join(", "),
                        root.display()
                    );
                    return ExitCode::FAILURE;
                }
                let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let dest = root.with_file_name(format!("{name}.{}", branch.replace('/', "-")));
                if let Err(e) = git::create_worktree(&root, &branch, &dest) {
                    eprintln!("keyjutsu: {e}");
                    return ExitCode::FAILURE;
                }
                println!("Running in a new worktree, {}, on branch {branch}.", dest.display());
                println!("Your working tree and its uncommitted changes are not touched.");
                start = dest;
            }
            Isolation::Branch => {
                if let Err(e) = git::create_branch(&root, &branch) {
                    eprintln!("keyjutsu: {e}");
                    return ExitCode::FAILURE;
                }
                println!("Switched to a new branch, {branch}. Your uncommitted changes came with it.");
            }
        }
    }
    options.cwd = Some(start.clone());
    let git_dir = git_folder(args.snapshot);
    // Recorded once the shell is running, from where it really is: a profile
    // can change folder after KeyJutsu starts it.
    let baseline: Arc<Mutex<Vec<git::RepoState>>> = Arc::default();
    let isolated_in = args.isolate.map(|_| start.clone());

    let result: Arc<Mutex<Option<(Outcome, Checkpoint)>>> = Arc::new(Mutex::new(None));
    let (snap, out, mode) = (snapshot.clone(), result.clone(), args.mode);
    // Administrator steps (§26): one UAC prompt, now, before the performance,
    // for a broker pinned to this snapshot. Never in the middle of a run.
    let needs_admin = snapshot
        .plan()
        .steps
        .iter()
        .any(|s| s.privilege == Some(keyjutsu_core::plan::model::Privilege::Administrator));
    let mut elevated_runner: Option<Arc<dyn keyjutsu_core::elevation::ElevatedRunner>> = None;
    if needs_admin && !keyjutsu_core::elevation::is_elevated() {
        let Some(exe) = keyjutsu_broker::broker_path() else {
            eprintln!(
                "keyjutsu: this plan has Administrator steps, and keyjutsu-broker.exe is not installed next to KeyJutsu"
            );
            return ExitCode::FAILURE;
        };
        println!(
            "This plan has Administrator steps. Windows will ask once, now, to start KeyJutsu's broker."
        );
        match keyjutsu_broker::launch(&exe, args.snapshot, snapshot.snapshot_hash()) {
            Ok(client) => elevated_runner = Some(Arc::new(client)),
            Err(e) => {
                eprintln!("keyjutsu: the elevation broker did not start: {e}. Nothing ran.");
                return ExitCode::FAILURE;
            }
        }
    }
    let exec_options = ExecuteOptions {
        mode,
        checkpoint: Some(cp_path.clone()),
        settled,
        resume_gate,
        elevated_runner,
        ..ExecuteOptions::default()
    };
    let (plan_for_git, git_dir_c, baseline_c, start_c) =
        (snapshot.plan().clone(), git_dir.clone(), baseline.clone(), start.clone());
    let controller: console::Controller = Box::new(move |session, events| {
        let here = session.shell_location().unwrap_or(start_c);
        // Isolation means nothing if the shell is not in the isolated tree.
        if let Some(dir) = &isolated_in
            && !git::same_path(&here, dir)
        {
            let reason = format!(
                "the shell started in {} instead of the isolated {} (a profile that changes folder does this; use --clean)",
                here.display(),
                dir.display()
            );
            if let Ok(mut r) = out.lock() {
                *r = Some((Outcome::Blocked { reason }, Checkpoint::new(snap.snapshot_hash())));
            }
            session.close();
            return;
        }
        let mut recorded = Vec::new();
        for (i, repo) in git::repositories(&here, &plan_for_git).iter().enumerate() {
            match git::record(repo, Some(&git_dir_c.join(i.to_string()))) {
                Ok(r) => recorded.push(r),
                Err(e) => eprintln!("keyjutsu: cannot record {} before the run: {e}", repo.display()),
            }
        }
        if !recorded.is_empty() {
            let _ = std::fs::create_dir_all(&git_dir_c).and_then(|()| {
                std::fs::write(
                    git_dir_c.join("baseline.json"),
                    serde_json::to_string_pretty(&recorded).unwrap_or_default(),
                )
            });
        }
        if let Ok(mut b) = baseline_c.lock() {
            *b = recorded;
        }
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
            // What the broker's elevated shell printed, shown in the terminal.
            ExecutionEvent::ElevatedOutput { text, .. } => {
                use std::io::Write;
                let mut out = std::io::stdout();
                let _ = out.write_all(text.replace('\n', "\r\n").as_bytes());
                let _ = out.flush();
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
    let mut git_reports = Vec::new();
    let baseline = baseline.lock().map(|b| b.clone()).unwrap_or_default();
    for (i, before) in baseline.iter().enumerate() {
        match git::report(before, &git_dir.join(i.to_string())) {
            Ok(r) => {
                print_git(&r, args.snapshot);
                git_reports.push(r);
            }
            Err(e) => eprintln!("keyjutsu: cannot compare {} after the run: {e}", before.root),
        }
    }
    let Some((outcome, checkpoint)) = result.lock().ok().and_then(|mut r| r.take()) else {
        println!("The shell ended before the plan finished. Checkpoint: {}", cp_path.display());
        return ExitCode::FAILURE;
    };
    if !args.ephemeral {
        let finished = fingerprint::now_rfc3339();
        let record = keyjutsu_core::history::SessionRecord {
            id: keyjutsu_core::store::new_id(&finished),
            started_at: started.clone(),
            finished_at: finished,
            task: snapshot.plan().title.clone().unwrap_or_else(|| snapshot.plan().task_id.clone()),
            agent: snapshot.plan().agent.clone(),
            snapshot: snapshot.to_json(),
            checkpoint: Some(checkpoint.clone()),
            outcome: outcome.clone(),
            git: git_reports,
        };
        match keyjutsu_core::store::Store::open(&keyjutsu_core::store::default_root())
            .and_then(|s| keyjutsu_core::history::save(&s, &record))
        {
            Ok(()) => println!("Recorded as session {} in the encrypted history.", record.id),
            Err(e) => eprintln!("keyjutsu: the session was not recorded: {e}"),
        }
    }
    for run in &checkpoint.runs {
        let mark = if run.succeeded { "ok    " } else { "FAILED" };
        println!("  {mark} {}", run.step);
        for c in run.checks.iter().filter(|c| c.passed != Some(true)) {
            println!("         {}: {}", c.check, c.detail);
        }
    }
    match outcome {
        Outcome::Boundary { phase, boundary } => {
            let what = keyjutsu_core::boundary::describe(boundary);
            println!();
            println!("Phase `{phase}` is done. The plan now waits for a {what}.");
            println!(
                "  {}",
                match boundary {
                    keyjutsu_core::plan::model::Boundary::WindowsRestart =>
                        "Restart Windows when you are ready; KeyJutsu does not restart it for you.",
                    keyjutsu_core::plan::model::Boundary::SignOut => "Sign out of Windows and back in.",
                    keyjutsu_core::plan::model::Boundary::ShellRestart =>
                        "The shell has ended; resume in a new terminal.",
                    keyjutsu_core::plan::model::Boundary::WslRestart =>
                        "Restart WSL (for example `wsl --shutdown`).",
                    keyjutsu_core::plan::model::Boundary::DockerRestart =>
                        "Restart Docker Desktop. KeyJutsu cannot see this one happen, so it takes your word.",
                }
            );
            println!("Then: keyjutsu run {} --resume", args.snapshot.display());
            ExitCode::SUCCESS
        }
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
            println!("Nothing has been rolled back. Your choices:");
            println!("  Diagnose first: the shell is as the step left it.");
            println!("  Review the recovery plan: `keyjutsu recover {}`", args.snapshot.display());
            println!("  Roll back: the same, with --confirm.");
            println!("  Stop here without rolling back: do nothing.");
            println!();
            println!(
                "To repair instead: `keyjutsu plan revise` with your guidance, validate and approve again, then"
            );
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
