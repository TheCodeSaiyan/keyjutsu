//! Collecting the environment fingerprint an approval is given against, and
//! the clock readings approvals are stamped with.
//!
//! The fingerprint records what the plan's commands will meet: the Windows
//! build, the architecture, each shell's path and version, and where each
//! executable the plan names resolves. It deliberately does not record tool
//! versions yet: running a tool to ask its version is validation's job
//! (Milestone 5), which will fill them in.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use keyjutsu_plan::Plan;
use keyjutsu_plan::hash::{EnvironmentFingerprint, FingerprintEntry};
use keyjutsu_terminal::{ShellKind, shell};

use crate::readiness;

fn shell_name(kind: ShellKind) -> &'static str {
    match kind {
        ShellKind::Pwsh => "pwsh",
        ShellKind::WindowsPowershell => "windows_powershell",
        ShellKind::Cmd => "cmd",
    }
}

/// Every executable a plan names, plan-wide and per step, without repeats.
pub fn named_executables(plan: &Plan) -> Vec<String> {
    let mut names: Vec<String> = plan
        .requirements
        .iter()
        .chain(plan.steps.iter().flat_map(|s| s.tool_requirements.iter()))
        .map(|r| r.executable.clone())
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Where `executable` resolves, as the shell would find it: an explicit path
/// as given, otherwise the first match on `PATH`, trying `PATHEXT`
/// extensions when the name has none.
pub fn resolve_executable(executable: &str) -> Option<PathBuf> {
    let given = Path::new(executable);
    if given.components().count() > 1 || given.is_absolute() {
        return given.is_file().then(|| given.to_path_buf());
    }
    let extensions: Vec<String> = if given.extension().is_some() {
        vec![String::new()]
    } else {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
            .split(';')
            .filter(|e| !e.is_empty())
            .map(|e| e.to_ascii_lowercase())
            .collect()
    };
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|dir| {
        extensions.iter().map(|ext| dir.join(format!("{executable}{ext}"))).find(|p| p.is_file())
    })
}

/// The fingerprint of this machine, for `plan` if given.
pub fn collect(plan: Option<&Plan>) -> EnvironmentFingerprint {
    let windows = readiness::windows_info();
    let os = match &windows.display_version {
        Some(v) => format!("{} {v}", windows.product),
        None => windows.product.clone(),
    };
    let shells = shell::detect_all()
        .into_iter()
        .map(|s| FingerprintEntry { name: shell_name(s.kind).into(), path: Some(s.path), version: s.version })
        .collect();
    let tools = plan
        .map(named_executables)
        .unwrap_or_default()
        .into_iter()
        .map(|name| FingerprintEntry {
            path: resolve_executable(&name).map(|p| p.display().to_string()),
            name,
            version: None,
        })
        .collect();
    EnvironmentFingerprint {
        os,
        build: windows.build.unwrap_or_default(),
        architecture: readiness::architecture(),
        shells,
        tools,
    }
}

/// The current time in RFC 3339, UTC, to the second.
pub fn now_rfc3339() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    rfc3339(secs)
}

/// Seconds since the Unix epoch as `YYYY-MM-DDTHH:MM:SSZ`. Howard Hinnant's
/// civil-from-days algorithm, to avoid a date library for one format.
pub fn rfc3339(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rem / 3_600, rem % 3_600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_known_instants() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z", "a leap day");
        assert_eq!(rfc3339(1_790_294_400), "2026-09-25T00:00:00Z");
        assert_eq!(rfc3339(4_107_542_399), "2100-02-28T23:59:59Z", "2100 is not a leap year");
    }

    #[test]
    fn resolves_executables_the_way_the_shell_would() {
        let cmd = resolve_executable("cmd").expect("cmd.exe is on PATH on Windows");
        assert!(cmd.to_string_lossy().to_ascii_lowercase().ends_with("cmd.exe"));
        assert_eq!(resolve_executable("cmd.exe"), Some(cmd));
        assert!(resolve_executable("keyjutsu-no-such-tool").is_none());
        assert!(resolve_executable(r"C:\no\such\tool.exe").is_none());
    }
}
