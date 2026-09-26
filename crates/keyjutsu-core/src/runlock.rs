//! One machine-changing run at a time.
//!
//! Two plans changing one machine at once can each invalidate what the other
//! validated, and a recovery can undo what another run is relying on. So a
//! run or a recovery holds this lock throughout, and `execute` and `recover`
//! take it as a parameter: neither can be called without it. Planning,
//! validation and everything read-only carry on alongside.
//!
//! It is a machine-wide semaphore, not a file, so it cannot be left behind:
//! Windows destroys it when the last handle closes, including when the
//! process holding it crashes.

/// The lock every front end takes. `KEYJUTSU_RUN_LOCK` names another, as
/// `KEYJUTSU_STORE` names another store: for tests that run KeyJutsu side by
/// side, never for a real run.
const MACHINE: &str = r"Global\KeyJutsu.run";

/// Held while a plan runs or recovers. Dropping it lets the next one start.
#[derive(Debug)]
pub struct RunLock {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
}

// SAFETY: the handle is a kernel semaphore handle, usable from any thread;
// a semaphore, unlike a mutex, is not owned by the thread that took it.
#[cfg(windows)]
#[allow(unsafe_code)]
unsafe impl Send for RunLock {}

impl RunLock {
    /// Take the machine's run lock, or say why not. Never waits: the
    /// operator decides what to do about the other run.
    pub fn take() -> Result<RunLock, String> {
        match std::env::var("KEYJUTSU_RUN_LOCK") {
            Ok(name) if !name.is_empty() => Self::take_named(&name),
            _ => Self::take_named(MACHINE),
        }
    }

    /// A lock no other run shares, for tests and trials: they run side by
    /// side, and must neither wait on each other nor on a real run.
    pub fn unshared() -> RunLock {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        #[allow(clippy::expect_used)]
        Self::take_named(&format!(r"Local\keyjutsu-unshared-{}-{n}", std::process::id()))
            .expect("a lock of its own name is never held")
    }

    /// A lock of the caller's naming.
    #[cfg(windows)]
    #[allow(unsafe_code)]
    pub fn take_named(name: &str) -> Result<RunLock, String> {
        use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, GetLastError, WAIT_OBJECT_0};
        use windows_sys::Win32::System::Threading::{CreateSemaphoreW, WaitForSingleObject};
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: `wide` is NUL-terminated and outlives the call; null
        // attributes give the object this account's default security.
        let handle = unsafe { CreateSemaphoreW(std::ptr::null(), 1, 1, wide.as_ptr()) };
        if handle.is_null() {
            // SAFETY: reads this thread's last error, set by the call above.
            let error = unsafe { GetLastError() };
            return Err(if error == ERROR_ACCESS_DENIED {
                "KeyJutsu is already changing this machine for another account, or as Administrator; \
                 wait for that run to finish"
                    .into()
            } else {
                format!("the run lock could not be made (Windows error {error})")
            });
        }
        // SAFETY: `handle` is the valid semaphore handle created above.
        if unsafe { WaitForSingleObject(handle, 0) } == WAIT_OBJECT_0 {
            return Ok(RunLock { handle });
        }
        // SAFETY: `handle` is valid and not used after this.
        unsafe { CloseHandle(handle) };
        Err("another KeyJutsu run is changing this machine; wait for it to finish, then try again".into())
    }

    #[cfg(not(windows))]
    pub fn take_named(_name: &str) -> Result<RunLock, String> {
        Ok(RunLock {})
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
impl Drop for RunLock {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::ReleaseSemaphore;
        // SAFETY: the handle is valid until closed here, and this lock took
        // the semaphore's one count, so giving it back cannot overflow it.
        unsafe {
            ReleaseSemaphore(self.handle, 1, std::ptr::null_mut());
            CloseHandle(self.handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(n: &str) -> String {
        format!(r"Local\keyjutsu-test-{}-{n}", std::process::id())
    }

    #[test]
    fn one_run_at_a_time_and_the_next_once_it_ends() {
        let n = name("one");
        let first = RunLock::take_named(&n).unwrap();
        let second = RunLock::take_named(&n).unwrap_err();
        assert!(second.contains("another KeyJutsu run"), "{second}");
        drop(first);
        assert!(RunLock::take_named(&n).is_ok(), "free again once the first ends");
    }

    #[test]
    fn a_lock_given_back_on_another_thread_is_free_again() {
        let n = name("thread");
        let held = RunLock::take_named(&n).unwrap();
        std::thread::spawn(move || drop(held)).join().unwrap();
        assert!(RunLock::take_named(&n).is_ok());
    }

    #[test]
    fn a_machine_wide_lock_needs_no_administrator() {
        // Every signed-in session sees a Global name; making one is not
        // reserved to Administrators the way a Global file mapping is.
        let n = format!(r"Global\keyjutsu-test-{}", std::process::id());
        let held = RunLock::take_named(&n).unwrap();
        assert!(RunLock::take_named(&n).is_err());
        drop(held);
    }

    #[test]
    fn different_locks_do_not_wait_on_each_other() {
        let _a = RunLock::take_named(&name("a")).unwrap();
        assert!(RunLock::take_named(&name("b")).is_ok());
    }
}
