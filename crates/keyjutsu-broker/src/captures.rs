//! What an Administrator step changes, captured and restored by the broker
//! itself (ADR 0017).
//!
//! Captures for recovery are usually kept beside the run's checkpoint, in
//! the operator's profile. For an Administrator step that will not do: the
//! broker would be writing back, as Administrator, values that anything
//! running as the operator could have forged. So the broker captures what
//! such a step declares, keeps it in `%ProgramData%\KeyJutsu\captures`, and
//! restores only from there.
//!
//! That folder must be one only Administrators can write. The broker makes
//! `%ProgramData%\KeyJutsu` with an access list of Administrators and SYSTEM
//! the first time, and every time checks that its owner is one of the two
//! and that its access list admits nobody else. An ordinary user may create
//! folders in `%ProgramData%`, so one could make `KeyJutsu` there first and
//! own it; the check refuses such a folder rather than using it.

#![allow(unsafe_code)]

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use keyjutsu_core::execute::CheckResult;
use keyjutsu_core::plan::ApprovedSnapshot;
use keyjutsu_core::plan::model::{CaptureKind, Step};
use keyjutsu_core::recovery::{Captured, StepCapture};

/// Owner Administrators, and full control for Administrators and SYSTEM
/// only, inherited by everything inside, nothing from the parent.
const SECURED: &str = "O:BAD:P(A;OICI;FA;;;BA)(A;OICI;FA;;;SY)";
/// Captures older than this are removed when the broker starts.
pub const KEEP_FOR: Duration = Duration::from_secs(30 * 24 * 60 * 60);

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide(p: *const u16) -> String {
    // SAFETY: callers pass a NUL-terminated wide string from Windows.
    unsafe {
        let mut n = 0;
        while *p.add(n) != 0 {
            n += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
    }
}

/// `%ProgramData%`, asked of Windows: the environment is the operator's to set.
fn program_data() -> Result<PathBuf, String> {
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{FOLDERID_ProgramData, SHGetKnownFolderPath};
    let mut p: *mut u16 = std::ptr::null_mut();
    // SAFETY: a known folder id and an out pointer; the string allocated on
    // success is freed below with CoTaskMemFree, as the call requires.
    let hr = unsafe { SHGetKnownFolderPath(&FOLDERID_ProgramData, 0, std::ptr::null_mut(), &mut p) };
    if hr != 0 || p.is_null() {
        return Err("cannot find the ProgramData folder".into());
    }
    let path = from_wide(p);
    // SAFETY: allocated by SHGetKnownFolderPath; not used after.
    unsafe { CoTaskMemFree(p.cast()) };
    Ok(PathBuf::from(path))
}

/// The folder's owner and access list, as SDDL.
fn security_of(path: &Path) -> Result<String, String> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW, GetNamedSecurityInfoW, SDDL_REVISION_1,
        SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{
        DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
    };
    let name = wide(&path.display().to_string());
    let info = OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
    let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: `name` is NUL-terminated; only the descriptor out pointer is
    // asked for, and it is freed below.
    let err = unsafe {
        GetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            info,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut sd,
        )
    };
    if err != 0 {
        return Err(format!("cannot read the access list of {}: error {err}", path.display()));
    }
    let mut text: *mut u16 = std::ptr::null_mut();
    // SAFETY: `sd` is the descriptor read above; the string allocated on
    // success is freed below.
    let ok = unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            sd,
            SDDL_REVISION_1,
            info,
            &mut text,
            std::ptr::null_mut(),
        )
    };
    // SAFETY: allocated by GetNamedSecurityInfoW; not used after.
    unsafe { LocalFree(sd) };
    if ok == 0 {
        return Err("cannot read the folder's access list".into());
    }
    let sddl = from_wide(text);
    // SAFETY: allocated by the conversion above; not used after.
    unsafe { LocalFree(text.cast()) };
    Ok(sddl)
}

/// Whether an owner and access list, as SDDL, keep a folder to
/// Administrators and SYSTEM: one of them owns it, and every entry that
/// allows anything names one of them. Anything else is refused, including
/// an entry that only allows reading, rather than judging which rights are
/// harmless.
pub fn check_secured(sddl: &str) -> Result<(), String> {
    let trusted = |sid: &str| matches!(sid, "BA" | "SY" | "S-1-5-32-544" | "S-1-5-18");
    let owner = sddl.strip_prefix("O:").and_then(|r| r.split("D:").next()).unwrap_or("");
    if !trusted(owner) {
        return Err(format!(
            "it is owned by {owner}, not by Administrators or SYSTEM, so someone else may have made it"
        ));
    }
    let Some(dacl) = sddl.split_once("D:").map(|(_, d)| d) else {
        return Err("it has no access list".into());
    };
    if dacl.starts_with("NO_ACCESS_CONTROL") {
        return Err("it has no access list, so anyone may write to it".into());
    }
    for ace in dacl.split('(').skip(1) {
        let fields: Vec<&str> = ace.trim_end_matches(')').split(';').collect();
        let (kind, sid) = (fields.first().copied().unwrap_or(""), fields.get(5).copied().unwrap_or(""));
        if kind.starts_with('A') && !trusted(sid) {
            return Err(format!("its access list also admits {sid}"));
        }
    }
    Ok(())
}

/// `%ProgramData%\KeyJutsu`, made the first time and checked every time.
pub fn secured_root() -> Result<PathBuf, String> {
    secured_root_in(&program_data()?)
}

/// `KeyJutsu` in `base`, made the first time and checked every time.
pub fn secured_root_in(base: &Path) -> Result<PathBuf, String> {
    let root = base.join("KeyJutsu");
    // Made here if it is not there; if it is, by whoever, it is checked.
    if let Err(e) = crate::protected::create_with(&root, SECURED)
        && !root.is_dir()
    {
        return Err(e);
    }
    // A junction here would be checked, and written through, as the folder
    // it points at: one someone else made is refused however well it is
    // locked down.
    {
        use std::os::windows::fs::MetadataExt;
        const REPARSE_POINT: u32 = 0x400;
        let link = std::fs::symlink_metadata(&root).map_err(|e| e.to_string())?;
        if link.file_attributes() & REPARSE_POINT != 0 {
            return Err(format!(
                "{} is a link to another folder, so it cannot hold Administrator captures. Remove it, and KeyJutsu's broker will make it again",
                root.display()
            ));
        }
    }
    check_secured(&security_of(&root)?).map_err(|why| {
        format!(
            "{} cannot hold Administrator captures: {why}. Remove it, and KeyJutsu's broker will make it again",
            root.display()
        )
    })?;
    Ok(root)
}

fn step_dir(root: &Path, snapshot: &ApprovedSnapshot, step: &Step) -> PathBuf {
    root.join("captures").join(snapshot.snapshot_hash()).join(&step.id)
}

/// Capture what `step` declares, before it runs, and keep it. Replaces any
/// earlier capture of the same step of the same snapshot.
pub fn capture(
    root: &Path,
    snapshot: &ApprovedSnapshot,
    step: &Step,
    at: String,
) -> Result<StepCapture, String> {
    let hash = snapshot.step_hashes().get(&step.id).cloned().unwrap_or_default();
    let dir = step_dir(root, snapshot, step);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let captured = keyjutsu_core::recovery::capture_step(
        step,
        &hash,
        keyjutsu_core::recovery::Backups::plain(&dir),
        at,
    )?;
    let text = serde_json::to_string_pretty(&captured).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("capture.json"), text).map_err(|e| e.to_string())?;
    Ok(captured)
}

/// What a captured item is of, to compare with what the step declared.
fn declared_as(item: &Captured) -> (CaptureKind, &str) {
    match item {
        Captured::File { path, .. } => (CaptureKind::File, path),
        Captured::RegistryValue { target, .. } => (CaptureKind::RegistryValue, target),
        Captured::ServiceState { name, .. } => (CaptureKind::ServiceState, name),
        Captured::PackageVersion { name } => (CaptureKind::PackageVersion, name),
    }
}

/// Put back what the broker captured for `step`, and nothing it did not
/// declare. The capture is removed once every item is back as it was.
pub fn restore(root: &Path, snapshot: &ApprovedSnapshot, step: &Step) -> Result<Vec<CheckResult>, String> {
    let dir = step_dir(root, snapshot, step);
    let text = std::fs::read_to_string(dir.join("capture.json"))
        .map_err(|_| format!("the broker captured nothing for `{}` in this plan", step.id))?;
    let captured: StepCapture = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let hash = snapshot.step_hashes().get(&step.id).cloned().unwrap_or_default();
    if captured.step != step.id || captured.step_hash != hash {
        return Err(format!("the capture kept for `{}` is not of the approved step", step.id));
    }
    let declared = step.recovery.as_ref().map(|r| r.capture.as_slice()).unwrap_or_default();
    for item in &captured.items {
        let (kind, target) = declared_as(item);
        if !declared.iter().any(|d| d.kind == kind && d.target == target) {
            return Err(format!("`{target}` was not declared by `{}`, so it is not restored", step.id));
        }
    }
    let checks =
        keyjutsu_core::recovery::restore_capture(&captured, keyjutsu_core::recovery::Backups::plain(&dir));
    if checks.iter().all(|c| c.passed != Some(false)) {
        let _ = std::fs::remove_dir_all(&dir);
    }
    Ok(checks)
}

/// Remove captures kept longer than `keep`, and the folders they leave empty.
pub fn sweep(root: &Path, keep: Duration) {
    let Ok(snapshots) = std::fs::read_dir(root.join("captures")) else { return };
    for snap in snapshots.flatten() {
        if let Ok(steps) = std::fs::read_dir(snap.path()) {
            for step in steps.flatten() {
                let old = std::fs::metadata(step.path().join("capture.json"))
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| SystemTime::now().duration_since(t).ok())
                    .is_none_or(|age| age > keep);
                if old {
                    let _ = std::fs::remove_dir_all(step.path());
                }
            }
        }
        let _ = std::fs::remove_dir(snap.path()); // Only if now empty.
    }
}

#[cfg(test)]
mod tests {
    use super::check_secured;

    #[test]
    fn only_a_folder_kept_to_administrators_and_system_holds_captures() {
        // As the broker makes it, and as Windows writes it back.
        check_secured("O:BAD:P(A;OICI;FA;;;BA)(A;OICI;FA;;;SY)").unwrap();
        check_secured("O:SYD:PAI(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)").unwrap();
        // Made first by an ordinary user, who owns it.
        let owned = check_secured("O:S-1-5-21-1-2-3-1001D:P(A;OICI;FA;;;BA)(A;OICI;FA;;;SY)").unwrap_err();
        assert!(owned.contains("owned by"), "{owned}");
        // Inheriting ProgramData's entries for users and creators.
        let users = check_secured("O:BAD:AI(A;OICIID;FA;;;SY)(A;OICIID;FA;;;BA)(A;OICIID;0x1200a9;;;BU)")
            .unwrap_err();
        assert!(users.contains("BU"), "{users}");
        assert!(check_secured("O:BAD:(A;;FA;;;BA)(A;OICIIO;GA;;;CO)").is_err());
        // No access list at all admits everyone.
        assert!(check_secured("O:BAD:NO_ACCESS_CONTROL").is_err());
        // A deny entry for someone else is fine: it only takes away.
        check_secured("O:BAD:P(D;;FA;;;BU)(A;OICI;FA;;;BA)(A;OICI;FA;;;SY)").unwrap();
    }

    #[test]
    fn a_junction_planted_where_captures_are_kept_is_refused() {
        let base = std::env::temp_dir().join(format!("kj-root-{}", std::process::id()));
        let elsewhere = base.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        let made = keyjutsu_core::terminal::shell::command("cmd")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(base.join("KeyJutsu"))
            .arg(&elsewhere)
            .output()
            .unwrap();
        assert!(made.status.success());
        let refused = super::secured_root_in(&base).unwrap_err();
        assert!(refused.contains("link to another folder"), "{refused}");
        let _ = std::fs::remove_dir_all(&base);
    }
}
