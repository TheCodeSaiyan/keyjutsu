//! Real elevation through the broker, for a disposable
//! machine such as Windows Sandbox. It writes a key under HKLM, which only
//! an Administrator can.
//!
//! `broker_trial DIR` starts `keyjutsu-broker.exe` (from the same folder)
//! through Windows' own "run as administrator" path, runs an approved
//! Administrator step through it, checks the HKLM key exists, and checks an
//! altered step is refused. Then a second Administrator step reads a staged
//! download: the trial checks what it read reached HKLM, that the folder it
//! read it from admitted only Administrators and SYSTEM and was removed
//! afterwards, and that a staged file changed since it was pinned is
//! refused. Everything it sees goes to standard output.

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

const PAYLOAD: &[u8] = b"approved payload";

/// A value under the trial's HKLM key, as the unelevated trial reads it.
fn reg_value(name: &str) -> Option<String> {
    let out = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-Command",
            &format!("(Get-ItemProperty -LiteralPath '{KEY}' -Name {name}).{name}"),
        ])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (out.status.success() && !text.is_empty()).then_some(text)
}

fn plan(value: &str) -> String {
    let sha = keyjutsu_core::plan::hash::sha256_hex(PAYLOAD);
    serde_json::json!({
        "schema_version": "1.0", "plan_id": "broker-trial", "task_id": "t",
        "target": {"id": "local", "kind": "local_windows"},
        "agent": {"name": "codex", "version": "1"},
        "steps": [{"id": "admin", "title": "Write under HKLM", "objective": "Needs Administrator.",
                   "kind": "command", "shell": {"kind": "windows_powershell"}, "privilege": "administrator",
                   "commands": [{"text": format!("New-Item -Path '{KEY}' -Force | Out-Null; Set-ItemProperty -Path '{KEY}' -Name Trial -Value {value}")}]},
                  {"id": "use-artifact", "title": "Record a download under HKLM",
                   "objective": "Needs Administrator and a staged file.",
                   "kind": "command", "shell": {"kind": "windows_powershell"}, "privilege": "administrator",
                   "artifacts": [{"name": "payload.txt", "source": "https://example.com/payload.txt", "sha256": sha}],
                   "commands": [{"text": format!(
                       "$f = $KJ_ARTIFACTS['payload.txt']; $d = Split-Path -Parent $f; \
                        New-Item -Path '{KEY}' -Force | Out-Null; \
                        Set-ItemProperty -Path '{KEY}' -Name Payload -Value (Get-Content -Raw -LiteralPath $f); \
                        Set-ItemProperty -Path '{KEY}' -Name Folder -Value $d; \
                        Set-ItemProperty -Path '{KEY}' -Name Access -Value (Get-Acl -LiteralPath $d).AccessToString")}]}]
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
    let client = match keyjutsu_broker::launch(
        &exe,
        &file,
        snap.snapshot_hash(),
        &keyjutsu_core::artifacts::default_store(),
    ) {
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

    let first = refusal.is_err() && run.is_ok() && exists;
    println!("first step: {}", if first { "ran elevated; the altered one was refused" } else { "FAILED" });

    // An artifact, staged in the operator's own store as `keyjutsu plan
    // stage` leaves it, which anything running as the operator can change.
    let step = snap.plan().step("use-artifact").unwrap().clone();
    let a = &step.artifacts[0];
    let staged = keyjutsu_core::artifacts::staged_path(
        &keyjutsu_core::artifacts::default_store(),
        a.sha256.as_ref().unwrap(),
        &a.name,
    );
    std::fs::create_dir_all(staged.parent().unwrap()).unwrap();
    std::fs::write(&staged, PAYLOAD).unwrap();
    let client = keyjutsu_broker::launch(
        &exe,
        &file,
        snap.snapshot_hash(),
        &keyjutsu_core::artifacts::default_store(),
    );
    let Ok(client) = client else {
        println!("FAIL: the broker did not start again");
        return ExitCode::FAILURE;
    };
    let run = client.run_step(snap.snapshot_hash(), "use-artifact", &snap.step_hashes()["use-artifact"]);
    println!("artifact step: {run:?}");
    let payload = reg_value("Payload");
    let folder = reg_value("Folder").unwrap_or_default();
    let access = reg_value("Access").unwrap_or_default();
    println!("read by the step: {payload:?}");
    println!("from: {folder}");
    println!("which admitted: {access:?}");
    let admins_only = access.contains("BUILTIN\\Administrators")
        && access.contains("NT AUTHORITY\\SYSTEM")
        && !access.contains("Users")
        && !access.contains(&std::env::var("USERNAME").unwrap_or_default());
    let removed = !folder.is_empty() && !std::path::Path::new(&folder).exists();
    let not_staged_copy =
        !folder.starts_with(&keyjutsu_core::artifacts::default_store().display().to_string());
    println!(
        "admins only: {admins_only}; removed afterwards: {removed}; not the operator's copy: {not_staged_copy}"
    );

    // Changed after it was pinned: refused, though the step is the approved one.
    std::fs::write(&staged, b"something else").unwrap();
    let tampered = client.run_step(snap.snapshot_hash(), "use-artifact", &snap.step_hashes()["use-artifact"]);
    println!("changed artifact: {tampered:?}");
    drop(client);

    let second = run.is_ok()
        && payload.as_deref() == Some("approved payload")
        && admins_only
        && removed
        && not_staged_copy
        && tampered.as_ref().is_err_and(|e| e.contains("changed since it was staged"));
    if first && second {
        println!(
            "PASS: the broker ran the approved Administrator steps elevated, refused the altered one, handed over a checked copy of the artifact from a folder only Administrators could write to, and refused a changed one"
        );
        ExitCode::SUCCESS
    } else {
        println!("FAIL");
        ExitCode::FAILURE
    }
}
