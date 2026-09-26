//! The first-run readiness scan.
//!
//! Non-invasive: it reads the registry and settings files, and starts each
//! shell once in a throwaway pseudo-console to prove, rather than assume, that
//! ConPTY works, that KeyJutsu's prompt marks survive the user's profile and
//! that staged typing reaches the input line intact. The probe command only
//! prints a word.

use std::sync::Arc;
use std::time::{Duration, Instant};

use keyjutsu_execution::{Cadence, ExecutionMode, PerformanceConfig, StagedScript, StagedStep, StepOutcome};
use keyjutsu_terminal::profile::{
    PowerShellProfileReport, TerminalProfile, detect_terminal_profile, inspect_powershell_profile,
};
use keyjutsu_terminal::{ProfileMode, ShellInfo, ShellKind, shell};
use serde::Serialize;

use crate::headless::Collector;
use crate::session::{Session, SessionEvent, SessionOptions};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CheckStatus {
    Ok,
    Warning,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export)]
pub struct Check {
    pub name: String,
    pub status: CheckStatus,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export)]
pub struct WindowsInfo {
    pub product: String,
    pub display_version: Option<String>,
    pub build: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export)]
pub struct ShellProbe {
    pub kind: ShellKind,
    pub profile: ProfileMode,
    /// The shell drew a prompt carrying KeyJutsu's marks.
    pub ready: bool,
    #[ts(type = "number | null")]
    pub ready_ms: Option<u64>,
    /// A command typed one character at a time ran and printed what it should.
    pub staged_typing: bool,
    /// What the shell reported for the probe command, where it can report one.
    pub exit_code: Option<i32>,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export)]
pub struct ReadinessReport {
    pub keyjutsu_version: String,
    pub windows: WindowsInfo,
    pub architecture: String,
    pub shells: Vec<ShellInfo>,
    pub probes: Vec<ShellProbe>,
    pub terminal_profile: TerminalProfile,
    pub powershell_profile: Option<PowerShellProfileReport>,
    pub checks: Vec<Check>,
}

pub fn windows_info() -> WindowsInfo {
    let output = std::process::Command::new("reg")
        .args(["query", r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    parse_windows_info(&output)
}

fn parse_windows_info(reg_output: &str) -> WindowsInfo {
    let value = |name: &str| -> Option<String> {
        reg_output.lines().find_map(|line| {
            let mut parts = line.split_whitespace();
            (parts.next() == Some(name)).then(|| {
                let _kind = parts.next();
                parts.collect::<Vec<_>>().join(" ")
            })
        })
    };
    let build_number: Option<u32> = value("CurrentBuild").and_then(|b| b.parse().ok());
    let revision = value("UBR").and_then(|u| u32::from_str_radix(u.trim_start_matches("0x"), 16).ok());
    let mut product = value("ProductName").unwrap_or_else(|| "Windows".into());
    // Windows 11 still reports "Windows 10" in ProductName; the build number
    // is what distinguishes them.
    if build_number.is_some_and(|b| b >= 22000) {
        product = product.replacen("Windows 10", "Windows 11", 1);
    }
    WindowsInfo {
        product,
        display_version: value("DisplayVersion"),
        build: build_number.map(|b| match revision {
            Some(r) => format!("{b}.{r}"),
            None => b.to_string(),
        }),
    }
}

pub fn architecture() -> String {
    let raw = std::env::var("PROCESSOR_ARCHITEW6432")
        .or_else(|_| std::env::var("PROCESSOR_ARCHITECTURE"))
        .unwrap_or_default();
    match raw.to_ascii_uppercase().as_str() {
        "AMD64" => "x64".into(),
        "ARM64" => "arm64".into(),
        "X86" => "x86".into(),
        "" => std::env::consts::ARCH.into(),
        other => other.to_ascii_lowercase(),
    }
}

/// A command whose output differs from its own text, so finding the output
/// proves the command ran rather than that the echo of the typing was seen.
fn probe_command(kind: ShellKind) -> (&'static str, &'static str) {
    match kind {
        ShellKind::Pwsh | ShellKind::WindowsPowershell => {
            ("Write-Output ('kj-probe' + '-ok')", "kj-probe-ok")
        }
        ShellKind::Cmd => ("echo kj-probe^-ok", "kj-probe-ok"),
    }
}

/// Start `kind` in a throwaway pseudo-console and type a harmless command
/// into it one character at a time.
pub fn probe_shell(kind: ShellKind, profile: ProfileMode, timeout: Duration) -> ShellProbe {
    let mut probe = ShellProbe {
        kind,
        profile,
        ready: false,
        ready_ms: None,
        staged_typing: false,
        exit_code: None,
        detail: String::new(),
    };
    let collector = Arc::new(Collector::new());
    let mut options = SessionOptions::new(kind);
    options.profile = profile;
    options.intercept_cursor_queries = true;
    let started = Instant::now();
    let session = match Session::open(options, collector.clone()) {
        Ok(s) => s,
        Err(e) => {
            probe.detail = format!("could not start: {e}");
            return probe;
        }
    };
    probe.ready = session.wait_ready(timeout);
    if !probe.ready {
        probe.detail = "no KeyJutsu prompt appeared; the profile may be replacing the prompt".into();
        session.close();
        return probe;
    }
    probe.ready_ms = Some(started.elapsed().as_millis() as u64);

    let (command, expected) = probe_command(kind);
    let script = StagedScript {
        steps: vec![StagedStep {
            id: "probe".into(),
            title: "Probe".into(),
            command: command.into(),
            mode: None,
            submit: None,
            answers: None,
        }],
    };
    let config = PerformanceConfig {
        mode: ExecutionMode::AutoPerformance,
        cadence: Cadence { base_ms: 0, variance_ms: 0, punctuation_pause_ms: 0, boundary_pause_ms: 0 },
        ..PerformanceConfig::default()
    };
    if let Err(e) = session.arm(script, config) {
        probe.detail = format!("could not arm: {e}");
        session.close();
        return probe;
    }
    let finished = collector
        .wait_until(timeout, |c| c.events.iter().any(|e| matches!(e, SessionEvent::StepFinished { .. })));
    let outcome = collector.snapshot().events.into_iter().find_map(|e| match e {
        SessionEvent::StepFinished { outcome, .. } => Some(outcome),
        _ => None,
    });
    session.close();

    probe.exit_code = match outcome {
        Some(StepOutcome::Succeeded { exit_code }) => Some(exit_code),
        Some(StepOutcome::Failed { exit_code }) => Some(exit_code),
        _ => None,
    };
    // ConPTY often moves the cursor instead of writing line breaks, so the
    // output is searched rather than split into lines. The typed command does
    // not contain `expected`, so finding it means the command really ran.
    let printed = collector.plain_output().contains(expected);
    probe.staged_typing = finished && printed && !matches!(outcome, Some(StepOutcome::Failed { .. }));
    probe.detail = match (finished, printed) {
        (false, _) => "the probe command did not finish in time".into(),
        (true, false) => {
            "the probe command ran but its output was not seen; something altered the typed text".into()
        }
        (true, true) if kind.reports_exit_codes() => "staged typing and exit codes both work".into(),
        (true, true) => "staged typing works; this shell cannot report exit codes".into(),
    };
    probe
}

pub fn scan() -> ReadinessReport {
    let windows = windows_info();
    let shells = shell::detect_all();
    let probe_timeout = Duration::from_secs(20);
    let probes: Vec<ShellProbe> = std::thread::scope(|scope| {
        let handles: Vec<_> = shells
            .iter()
            .map(|s| {
                let kind = s.kind;
                scope.spawn(move || probe_shell(kind, ProfileMode::Detected, probe_timeout))
            })
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    });
    let terminal_profile = detect_terminal_profile();
    let powershell_profile = shell::locate(ShellKind::Pwsh)
        .or_else(|| shell::locate(ShellKind::WindowsPowershell))
        .map(|p| inspect_powershell_profile(&p));

    let mut checks = Vec::new();
    let windows_ok = windows.product.contains("Windows 11");
    checks.push(Check {
        name: "Windows".into(),
        status: if windows_ok { CheckStatus::Ok } else { CheckStatus::Warning },
        detail: if windows_ok {
            format!("{} {}", windows.product, windows.display_version.clone().unwrap_or_default())
        } else {
            format!("{}: KeyJutsu V1 supports Windows 11 x64 only", windows.product)
        },
    });
    let arch = architecture();
    checks.push(Check {
        name: "Architecture".into(),
        status: if arch == "x64" { CheckStatus::Ok } else { CheckStatus::Warning },
        detail: arch.clone(),
    });
    let conpty_ok = probes.iter().any(|p| p.ready);
    checks.push(Check {
        name: "ConPTY".into(),
        status: if conpty_ok { CheckStatus::Ok } else { CheckStatus::Unavailable },
        detail: if conpty_ok {
            "a pseudo-console started and a shell drew its prompt through it".into()
        } else {
            "no shell could be started in a pseudo-console".into()
        },
    });
    for probe in &probes {
        checks.push(Check {
            name: format!("{} staged input", probe.kind.display_name()),
            status: if probe.staged_typing { CheckStatus::Ok } else { CheckStatus::Warning },
            detail: probe.detail.clone(),
        });
    }
    if let Some(report) = &powershell_profile
        && report.may_interfere()
    {
        let mut tools = Vec::new();
        if report.uses_oh_my_posh {
            tools.push("oh-my-posh");
        }
        if report.uses_starship {
            tools.push("Starship");
        }
        if report.configures_psreadline {
            tools.push("PSReadLine customisation");
        }
        checks.push(Check {
            name: "PowerShell profile".into(),
            status: CheckStatus::Warning,
            detail: format!(
                "uses {}; if a performance misbehaves, arm with the clean profile",
                tools.join(", ")
            ),
        });
    }
    checks.push(Check {
        name: "Telemetry".into(),
        status: CheckStatus::Ok,
        detail: "off: KeyJutsu has no telemetry, crash reporting or remote diagnostics".into(),
    });
    let agents: Vec<&'static str> =
        keyjutsu_agent::detect_all().into_iter().filter(|a| a.installed()).map(|a| a.name).collect();
    checks.push(if agents.is_empty() {
        Check {
            name: "AI agents".into(),
            status: CheckStatus::Warning,
            detail: "none installed: plans can still be opened from files".into(),
        }
    } else {
        Check { name: "AI agents".into(), status: CheckStatus::Ok, detail: agents.join(", ") }
    });
    let broker = std::env::current_exe().ok().map(|e| e.with_file_name("keyjutsu-broker.exe"));
    checks.push(match broker {
        Some(b) if b.exists() => Check {
            name: "Elevation broker".into(),
            status: CheckStatus::Ok,
            detail: "installed: Administrator steps run through it, after one UAC prompt before the run"
                .into(),
        },
        _ => Check {
            name: "Elevation broker".into(),
            status: CheckStatus::Warning,
            detail: "keyjutsu-broker.exe is not next to KeyJutsu, so Administrator steps cannot run".into(),
        },
    });
    let sample = b"keyjutsu readiness";
    let dpapi_works =
        crate::dpapi::protect(sample).and_then(|p| crate::dpapi::unprotect(&p)).is_ok_and(|u| u == sample);
    checks.push(Check {
        name: "Encrypted storage".into(),
        status: if dpapi_works { CheckStatus::Ok } else { CheckStatus::Unavailable },
        detail: if dpapi_works {
            "history is encrypted with a key only this Windows account can unlock (DPAPI)".into()
        } else {
            "Windows would not protect a key for this account, so history cannot be kept".into()
        },
    });

    ReadinessReport {
        keyjutsu_version: env!("CARGO_PKG_VERSION").into(),
        windows,
        architecture: arch,
        shells,
        probes,
        terminal_profile,
        powershell_profile,
        checks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_windows_11_from_a_registry_that_says_windows_10() {
        let reg = "HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\r\n    ProductName    REG_SZ    Windows 10 Pro\r\n    DisplayVersion    REG_SZ    25H2\r\n    CurrentBuild    REG_SZ    26200\r\n    UBR    REG_DWORD    0x24d1\r\n";
        let w = parse_windows_info(reg);
        assert_eq!(w.product, "Windows 11 Pro");
        assert_eq!(w.display_version.as_deref(), Some("25H2"));
        assert_eq!(w.build.as_deref(), Some("26200.9425"));
    }

    #[test]
    fn leaves_windows_10_alone_on_a_windows_10_build() {
        let reg = "    ProductName    REG_SZ    Windows 10 Pro\r\n    CurrentBuild    REG_SZ    19045\r\n";
        assert_eq!(parse_windows_info(reg).product, "Windows 10 Pro");
    }

    #[test]
    fn probe_commands_print_something_other_than_their_own_text() {
        for kind in ShellKind::ALL {
            let (command, expected) = probe_command(kind);
            assert!(!command.contains(expected), "{kind:?}");
        }
    }
}
