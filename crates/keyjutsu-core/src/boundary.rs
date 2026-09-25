//! Session boundaries (§32): a restart, sign-out, shell or WSL restart
//! between two phases of a plan.
//!
//! When a phase that ends at a boundary is done, execution stops and the
//! checkpoint records what should change: the boot time for a Windows
//! restart, the logon id for a sign-out, the shell's process id for a shell
//! restart, WSL's boot id for a WSL restart. Resuming checks that it did
//! change, compares the machine with the approved one again, checks that what
//! the earlier phases achieved still holds, and asks the operator before
//! anything more runs. Nothing from before the boundary is taken on trust.

use std::sync::Arc;
use std::time::Duration;

use keyjutsu_plan::hash::{Drift, EnvironmentFingerprint};
use keyjutsu_plan::model::{Boundary, Plan};
use serde::{Deserialize, Serialize};

use crate::execute::CheckResult;
use crate::session::Session;

/// A plan stopped at a boundary, waiting for it to be crossed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "execute/")]
pub struct BoundaryWait {
    pub after_phase: String,
    pub kind: Boundary,
    /// What should differ once the boundary has been crossed; `None` when
    /// KeyJutsu has no way to tell, so the operator's word is all there is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub identity: Option<String>,
    pub recorded_at: String,
}

/// What the operator is shown before resuming after a boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "execute/")]
pub struct BoundaryNotice {
    pub kind: Boundary,
    pub after_phase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub next_phase: Option<String>,
    /// KeyJutsu saw the boundary happen (`false` when it cannot tell).
    pub verified: bool,
    /// What the earlier phases achieved, checked again after the boundary.
    pub rechecked: Vec<CheckResult>,
    pub drifts: Vec<Drift>,
}

/// What identifies the far side of a boundary.
pub type BoundaryProbe = Arc<dyn Fn(Boundary, &Session) -> Option<String> + Send + Sync>;
/// Asked before resuming after a boundary; `true` to go on.
pub type ResumeGate = Arc<dyn Fn(&BoundaryNotice) -> bool + Send + Sync>;
/// This machine's environment now, for the plan.
pub type FingerprintNow = Arc<dyn Fn(&Plan) -> EnvironmentFingerprint + Send + Sync>;

pub fn describe(kind: Boundary) -> &'static str {
    match kind {
        Boundary::WindowsRestart => "Windows restart",
        Boundary::SignOut => "sign-out and sign-in",
        Boundary::ShellRestart => "shell restart",
        Boundary::WslRestart => "WSL restart",
        Boundary::DockerRestart => "Docker restart",
    }
}

fn run(program: &str, args: &[&str]) -> Option<String> {
    let mut c = std::process::Command::new(program);
    c.args(args);
    let out = keyjutsu_validation::process::run(c, "", Duration::from_secs(30)).ok()?;
    let text = out.stdout.trim().to_owned();
    (out.success && !text.is_empty()).then_some(text)
}

/// The real identity of each boundary's far side.
pub fn identity(kind: Boundary, session: &Session) -> Option<String> {
    match kind {
        // Windows PowerShell is on every Windows; PowerShell 7 may not be.
        Boundary::WindowsRestart => run(
            "powershell.exe",
            &[
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "(Get-CimInstance Win32_OperatingSystem).LastBootUpTime.ToUniversalTime().ToString('o')",
            ],
        ),
        // The logon SID (S-1-5-5-x-y) is new for every sign-in.
        Boundary::SignOut => run("whoami", &["/logonid"]),
        Boundary::ShellRestart => session.shell_pid().map(|p| p.to_string()),
        Boundary::WslRestart => run("wsl.exe", &["-e", "cat", "/proc/sys/kernel/random/boot_id"]),
        // Docker exposes nothing that changes on restart: the operator's word.
        Boundary::DockerRestart => None,
    }
}

pub fn real_probe() -> BoundaryProbe {
    Arc::new(identity)
}
