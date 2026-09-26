//! Session boundaries: a restart, sign-out, shell or WSL restart
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

use keyjutsu_plan::ApprovedSnapshot;

use crate::execute::{CheckResult, Checkpoint};
use crate::session::Session;
use crate::store::Store;

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

/// A run stopped at a boundary and not yet past it, found on disk: after a
/// Windows restart, nothing else remembers it.
#[derive(Debug, Clone)]
pub struct Waiting {
    pub snapshot: ApprovedSnapshot,
    pub snapshot_path: std::path::PathBuf,
    pub checkpoint: Checkpoint,
    pub checkpoint_path: std::path::PathBuf,
}

impl Waiting {
    pub fn wait(&self) -> Option<&BoundaryWait> {
        self.checkpoint.boundary.as_ref()
    }
}

/// The most recent of `checkpoints` that waits at a boundary, with the
/// snapshot beside it (`snapshot.json` in the same folder). Only a
/// checkpoint KeyJutsu wrote for this account, for a snapshot this account
/// approved, counts: a file planted or edited in the folder is passed over,
/// never offered to the operator.
pub fn find_waiting(
    store: &Store,
    checkpoints: impl IntoIterator<Item = std::path::PathBuf>,
) -> Option<Waiting> {
    let mut best: Option<Waiting> = None;
    for checkpoint_path in checkpoints {
        let Ok(checkpoint) = crate::approvals::load_checkpoint(store, &checkpoint_path) else { continue };
        let Some(wait) = checkpoint.boundary.clone() else { continue };
        let snapshot_path = checkpoint_path.with_file_name("snapshot.json");
        let Ok(text) = std::fs::read_to_string(&snapshot_path) else { continue };
        let Ok(snapshot) = ApprovedSnapshot::from_json(&text) else { continue };
        if snapshot.snapshot_hash() != checkpoint.snapshot_hash
            || crate::approvals::check_approval(store, &snapshot).is_err()
        {
            continue;
        }
        if best.as_ref().and_then(Waiting::wait).is_none_or(|b| wait.recorded_at > b.recorded_at) {
            best = Some(Waiting { snapshot, snapshot_path, checkpoint, checkpoint_path });
        }
    }
    best
}

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
