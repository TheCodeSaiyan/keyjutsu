//! Real elevation through the broker, for a disposable
//! machine such as Windows Sandbox. It writes a key under HKLM, which only
//! an Administrator can.
//!
//! `broker_trial DIR` starts `keyjutsu-broker.exe` (from the same folder)
//! through Windows' own "run as administrator" path, runs an approved
//! Administrator step through it, checks the HKLM key exists, and checks an
//! altered step is refused. Everything it sees goes to standard output.

#![allow(clippy::unwrap_used, clippy::print_stdout)] // A trial program, not a library.

use std::path::PathBuf;
use std::process::ExitCode;

use keyjutsu_core::elevation::{ElevatedRunner, is_elevated};
use keyjutsu_core::plan::model::Readiness;
use keyjutsu_core::plan::{ApprovalBook, ValidPlan, parse_plan, seal};
use keyjutsu_core::validation::{Options, validate};

const KEY: &str = r"HKLM:\SOFTWARE\KeyJutsuBrokerTrial";

fn key_exists() -> bool {
    std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-Command",
            &format!("if (Test-Path -LiteralPath '{KEY}') {{ exit 0 }} else {{ exit 1 }}"),
        ])
        .status()
        .is_ok_and(|s| s.success())
}

fn plan(value: &str) -> String {
    serde_json::json!({
        "schema_version": "1.0", "plan_id": "broker-trial", "task_id": "t",
        "target": {"id": "local", "kind": "local_windows"},
        "agent": {"name": "codex", "version": "1"},
        "steps": [{"id": "admin", "title": "Write under HKLM", "objective": "Needs Administrator.",
                   "kind": "command", "shell": {"kind": "windows_powershell"}, "privilege": "administrator",
                   "commands": [{"text": format!("New-Item -Path '{KEY}' -Force | Out-Null; Set-ItemProperty -Path '{KEY}' -Name Trial -Value {value}")}]}]
    })
    .to_string()
}

fn main() -> ExitCode {
    let Some(dir) = std::env::args().nth(1).map(PathBuf::from) else {
        println!("usage: broker_trial DIR");
        return ExitCode::from(2);
    };
    println!("this trial process elevated: {}", is_elevated());
    println!("HKLM key before: {}", key_exists());

    let draft = parse_plan(&plan("approved")).unwrap();
    let report = validate(&draft, Options { dry_run: false, broker_available: true });
    for (id, s) in &report.steps {
        println!("validated {id}: {:?}", s.readiness);
    }
    if report.steps.values().any(|s| s.readiness != Readiness::Ready) {
        println!("FAIL: not READY");
        return ExitCode::FAILURE;
    }
    let at = keyjutsu_core::fingerprint::now_rfc3339();
    let v = ValidPlan::revalidate(report.record_in(draft.plan(), &at), false).unwrap();
    let mut book = ApprovalBook::new();
    book.approve_all_except_critical(&v, &at);
    let snap = seal(&v, &book, None, &at).unwrap();
    let file = dir.join("broker-trial-snapshot.json");
    std::fs::write(&file, snap.to_json()).unwrap();

    let exe = std::env::current_exe().unwrap().with_file_name("keyjutsu-broker.exe");
    println!(
        "starting the broker through 'run as administrator' (a UAC prompt may appear in the Sandbox window)"
    );
    let client = match keyjutsu_broker::launch(&exe, &file, snap.snapshot_hash()) {
        Ok(c) => c,
        Err(e) => {
            println!("FAIL: the broker did not start: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!("broker says it is elevated: {}", client.elevated);

    // An altered version of the step is refused by the real broker too.
    let altered = parse_plan(&plan("altered")).unwrap();
    let altered_hash =
        keyjutsu_core::plan::hash::step_hashes(altered.plan(), altered.graph())["admin"].clone();
    let refusal = client.run_step(snap.snapshot_hash(), "admin", &altered_hash);
    println!("altered step: {refusal:?}");

    let run = client.run_step(snap.snapshot_hash(), "admin", &snap.step_hashes()["admin"]);
    println!("approved step: {run:?}");
    drop(client);
    let exists = key_exists();
    println!("HKLM key after: {exists}");

    let ok = refusal.is_err() && run.is_ok() && exists;
    if ok {
        println!("PASS: the broker ran the approved Administrator step elevated and refused the altered one");
        ExitCode::SUCCESS
    } else {
        println!("FAIL");
        ExitCode::FAILURE
    }
}
