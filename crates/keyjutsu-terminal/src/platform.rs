//! The one Win32 call this crate makes directly.
//!
//! Windows passes a process's "ignore Ctrl+C" attribute on to every child it
//! creates. A process started in a new process group gets that attribute set,
//! and many launchers (CI agents, IDE task runners, some terminals' "run"
//! commands) start programs that way. Without this, a shell KeyJutsu starts
//! would inherit it, and Ctrl+C typed into the pseudo-console would reach the
//! shell as a byte but never interrupt the running command. That was observed
//! directly: `Start-Sleep` ignored `0x03` until the attribute was cleared, and
//! stopped at once afterwards.

/// Stop ignoring Ctrl+C in this process, so shells started from it handle
/// Ctrl+C normally. Idempotent.
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn restore_ctrl_c_for_children() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // SAFETY: with a null handler routine, SetConsoleCtrlHandler only sets
        // or clears the calling process's Ctrl+C-ignore attribute. It takes no
        // pointers and has no memory-safety preconditions.
        unsafe {
            windows_sys::Win32::System::Console::SetConsoleCtrlHandler(None, 0);
        }
    });
}

#[cfg(not(windows))]
pub fn restore_ctrl_c_for_children() {}
