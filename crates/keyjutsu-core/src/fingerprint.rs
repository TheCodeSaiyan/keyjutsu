//! Collecting the environment fingerprint an approval is given against, and
//! the clock readings approvals are stamped with.
//!
//! The fingerprint records what the plan's commands will meet: the Windows
//! build, the architecture, each shell's path and version, and where each
//! executable the plan names resolves, with its version read from the file's
//! version resource. The tool itself is never run to ask.

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

pub use keyjutsu_validation::probe::resolve_executable;

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
    let names = plan.map(named_executables).unwrap_or_default();
    // One PowerShell lookup reads every tool's file version at once.
    let versions = shell::locate(ShellKind::Pwsh)
        .or_else(|| shell::locate(ShellKind::WindowsPowershell))
        .filter(|_| !names.is_empty())
        .and_then(|ps| {
            let refs: Vec<&str> = names.iter().map(String::as_str).collect();
            keyjutsu_validation::powershell::analyse(&ps, &[], &refs, &[]).ok()
        });
    let tools = names
        .into_iter()
        .map(|name| {
            let version = versions
                .as_ref()
                .and_then(|a| a.tools.get(&name))
                .and_then(Option::as_ref)
                .and_then(|t| t.file_version.clone());
            FingerprintEntry {
                path: resolve_executable(&name).map(|p| p.display().to_string()),
                name,
                version,
            }
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
