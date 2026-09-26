//! Real elevation through the broker, for a disposable
//! machine such as Windows Sandbox. It writes a key under HKLM, which only
//! an Administrator can.
//!
//! `broker_trial DIR` starts `keyjutsu-broker.exe` (from the same folder)
//! through Windows' own "run as administrator" path, runs an approved
//! Administrator step through it, checks the HKLM key exists, and checks an
//! altered step is refused, then undoes it through the broker with the
//! step's approved recovery command. A third has what it changes captured
//! by the broker and put back from the broker's own copy, after the broker
//! has refused a captures folder made first by an ordinary user. Then a
//! fourth Administrator step reads a staged
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

/// A folder any user can write to, where an Administrator step's file is.
const USER_DIR: &str = r"C:\Users\Public\KeyJutsuTrial";
/// A folder only Administrators can write to.
const PROTECTED: &str = r"C:\Program Files\KeyJutsuTrialProtected";

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
                   "reversibility": {"level": "full"},
                   "recovery": {"strategy": "commands",
                                "commands": [{"text": format!("Remove-ItemProperty -Path '{KEY}' -Name Trial")}]},
                   "commands": [{"text": format!("New-Item -Path '{KEY}' -Force | Out-Null; Set-ItemProperty -Path '{KEY}' -Name Trial -Value {value}")}]},
                  {"id": "admin-captured", "title": "Change a value the broker captured",
                   "objective": "Needs Administrator, and is put back from the broker's capture.",
                   "kind": "command", "shell": {"kind": "windows_powershell"}, "privilege": "administrator",
                   "reversibility": {"level": "full"},
                   "recovery": {"strategy": "restore_captured_state",
                                "capture": [{"kind": "registry_value", "target": format!("{KEY}\\Captured")}]},
                   "commands": [{"text": format!(
                       "$d = Join-Path $env:ProgramData 'KeyJutsu'; \
                        New-Item -Path '{KEY}' -Force | Out-Null; \
                        Set-ItemProperty -Path '{KEY}' -Name RootOwner -Value (Get-Acl -LiteralPath $d).Owner; \
                        Set-ItemProperty -Path '{KEY}' -Name RootAccess -Value (Get-Acl -LiteralPath $d).AccessToString; \
                        Set-ItemProperty -Path '{KEY}' -Name Captured -Value changed")}]},
                  {"id": "admin-file", "title": "Change a file the broker captured",
                   "objective": "Needs Administrator, and is put back from the broker's capture.",
                   "kind": "command", "shell": {"kind": "windows_powershell"}, "privilege": "administrator",
                   "reversibility": {"level": "full"},
                   "recovery": {"strategy": "restore_captured_state",
                                "capture": [{"kind": "file", "target": format!("{USER_DIR}\\app\\config.txt")}]},
                   "commands": [{"text": format!(
                       "New-Item -ItemType Directory -Force -Path '{PROTECTED}' | Out-Null; \
                        Set-Content -LiteralPath '{PROTECTED}\\config.txt' -Value precious -NoNewline; \
                        Set-Content -LiteralPath '{USER_DIR}\\app\\config.txt' -Value changed -NoNewline")}]},
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

    // Someone gets to %ProgramData%\KeyJutsu first, as an ordinary user may.
    let secured = std::path::PathBuf::from(std::env::var("ProgramData").unwrap()).join("KeyJutsu");
    let squatted = std::fs::create_dir(&secured).is_ok();
    println!("{} made first by this unelevated account: {squatted}", secured.display());

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
    let written = reg_value("Trial");
    println!("value written: {written:?}");
    // Undone through the broker: the step's approved recovery command, run
    // as Administrator, named by step and hash only.
    let recovery = client.recover_step(snap.snapshot_hash(), "admin", &snap.step_hashes()["admin"]);
    println!("recovery: {recovery:?}");
    let after_recovery = reg_value("Trial");
    println!("value after recovery: {after_recovery:?}");
    let undone = recovery.is_ok() && written.as_deref() == Some("approved") && after_recovery.is_none();

    // Captured by the broker, where only Administrators can write.
    let captured_hash = snap.step_hashes()["admin-captured"].clone();
    let squat_refused = client.run_step(snap.snapshot_hash(), "admin-captured", &captured_hash);
    println!("with the folder made by someone else: {squat_refused:?}");
    let _ = std::fs::remove_dir(&secured);
    let run_captured = client.run_step(snap.snapshot_hash(), "admin-captured", &captured_hash);
    println!("captured: {:?}", run_captured.as_ref().map(|r| &r.captured));
    let changed = reg_value("Captured");
    let root_owner = reg_value("RootOwner").unwrap_or_default();
    let root_access = reg_value("RootAccess").unwrap_or_default();
    println!("value after the step: {changed:?}; folder owner: {root_owner}; access: {root_access:?}");
    let planted = std::fs::write(secured.join("planted.json"), "{}");
    println!("this unelevated account writing into it: {planted:?}");
    let restored = client.restore_step(snap.snapshot_hash(), "admin-captured", &captured_hash);
    println!("restore: {restored:?}");
    let after_restore = reg_value("Captured");
    println!("value after restore: {after_restore:?}");
    let again = client.restore_step(snap.snapshot_hash(), "admin-captured", &captured_hash);
    println!("restore again: {again:?}");
    let captures_held = squatted
        && squat_refused.as_ref().is_err_and(|e| e.contains("cannot hold Administrator captures"))
        && run_captured.as_ref().is_ok_and(|r| r.captured.is_some())
        && changed.as_deref() == Some("changed")
        && root_owner.contains("Administrators")
        && !root_access.contains("Users")
        && planted.is_err()
        && restored.as_ref().is_ok_and(|c| !c.is_empty() && c.iter().all(|c| c.passed == Some(true)))
        && after_restore.is_none()
        && again.as_ref().is_err_and(|e| e.contains("captured nothing"));
    println!(
        "captures: {}",
        if captures_held {
            "refused a squatted folder, kept in an Administrators-only one, restored once"
        } else {
            "FAILED"
        }
    );

    // A file the broker captured, in a folder anyone can write to. Between
    // the step and the restore, the unelevated side swaps that folder for a
    // junction into Program Files: the broker, as Administrator, must not
    // write through it. Then, with the folder put back, it restores as normal.
    let app = std::path::Path::new(USER_DIR).join("app");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(app.join("config.txt"), "original").unwrap();
    let file_hash = snap.step_hashes()["admin-file"].clone();
    let file_run = client.run_step(snap.snapshot_hash(), "admin-file", &file_hash);
    println!("file step: {:?}", file_run.as_ref().map(|r| &r.captured));
    let moved = std::path::Path::new(USER_DIR).join("app-moved");
    std::fs::rename(&app, &moved).unwrap();
    let junction = std::process::Command::new("cmd")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&app)
        .arg(PROTECTED)
        .output()
        .unwrap();
    println!("junction to {PROTECTED} made by this unelevated account: {}", junction.status.success());
    let through = client.restore_step(snap.snapshot_hash(), "admin-file", &file_hash);
    println!("restore through the junction: {through:?}");
    let protected_after = std::fs::read_to_string(std::path::Path::new(PROTECTED).join("config.txt"));
    println!("{PROTECTED}\\config.txt after: {protected_after:?}");
    std::fs::remove_dir(&app).unwrap(); // The junction itself, not its target.
    std::fs::rename(&moved, &app).unwrap();
    let back = client.restore_step(snap.snapshot_hash(), "admin-file", &file_hash);
    println!("restore with the folder put back: {back:?}");
    let file_after = std::fs::read_to_string(app.join("config.txt"));
    println!("file after: {file_after:?}");
    let links_held = file_run.is_ok()
        && junction.status.success()
        && through.as_ref().is_ok_and(|c| {
            !c.is_empty()
                && c.iter().all(|c| c.passed == Some(false) && c.detail.contains("link or junction"))
        })
        && protected_after.as_deref().is_ok_and(|t| t == "precious")
        && back.as_ref().is_ok_and(|c| !c.is_empty() && c.iter().all(|c| c.passed == Some(true)))
        && file_after.as_deref().is_ok_and(|t| t == "original");
    println!(
        "links: {}",
        if links_held {
            "the broker would not write through a junction planted after its capture, and restored once it was gone"
        } else {
            "FAILED"
        }
    );
    drop(client);
    let exists = key_exists();
    println!("HKLM key after: {exists}");

    let first = refusal.is_err() && run.is_ok() && exists && undone;
    println!(
        "first step: {}",
        if first { "ran elevated, the altered one was refused, and it was undone" } else { "FAILED" }
    );

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
    if first && captures_held && links_held && second {
        println!(
            "PASS: the broker ran the approved Administrator steps elevated, refused the altered one, undid one with its approved recovery command, restored another from its own capture after refusing a squatted folder, would not restore a file through a planted junction, handed over a checked copy of the artifact from a folder only Administrators could write to, and refused a changed one"
        );
        ExitCode::SUCCESS
    } else {
        println!("FAIL");
        ExitCode::FAILURE
    }
}
