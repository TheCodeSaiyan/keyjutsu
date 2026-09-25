//! Administrator steps (§26). KeyJutsu itself runs unelevated; a step that
//! needs Administrator goes to an elevation broker started once, before the
//! run, which runs only approved steps bound to their hashes (ADR 0011).

use keyjutsu_execution::StepOutcome;
use serde::{Deserialize, Serialize};

/// What an elevated run of one approved step produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElevatedRun {
    /// One per staged line, as the terminal would have reported them.
    pub outcomes: Vec<StepOutcome>,
    /// What the elevated shell printed, for showing in the terminal.
    pub output: String,
}

/// Runs an approved Administrator step elevated. There is deliberately no
/// way to pass a command: only the snapshot, the step and the step's hash,
/// which the runner checks against its own copy.
pub trait ElevatedRunner: Send + Sync {
    fn run_step(&self, snapshot_hash: &str, step: &str, step_hash: &str) -> Result<ElevatedRun, String>;
}

/// Whether this process is elevated (its token is the full Administrator
/// token, not the filtered one UAC gives by default).
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn is_elevated() -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: GetCurrentProcess returns a pseudo-handle that needs no
    // closing; OpenProcessToken writes a real handle into `token` on
    // success, which is closed below.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return false;
    }
    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut len = 0u32;
    // SAFETY: `elevation` is a correctly sized TOKEN_ELEVATION buffer and
    // `len` receives the size written; the token handle is valid here.
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
    };
    // SAFETY: `token` was opened above and is not used after this.
    unsafe {
        CloseHandle(token);
    }
    ok != 0 && elevation.TokenIsElevated != 0
}

#[cfg(not(windows))]
pub fn is_elevated() -> bool {
    false
}
