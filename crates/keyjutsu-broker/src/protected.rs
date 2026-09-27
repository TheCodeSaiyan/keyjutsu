//! A folder only Administrators and SYSTEM can write to, for the copies of
//! a step's artifacts that the broker hands to it.
//!
//! Staged artifacts live in the operator's own profile, where anything
//! running as the operator can change them. An elevated step must not run
//! from there: a file swapped between its check and its use would be run as
//! Administrator. So the broker copies each one here and checks the copy,
//! which nothing unelevated can then touch. The folder sits in the Windows
//! folder's `Temp`, whose standard access list lets ordinary users create
//! things but not rename or delete what others made, under a random name so
//! it cannot be created in advance, and with a protected access list of its
//! own. It is removed when dropped.

#![allow(unsafe_code)]

use std::path::{Path, PathBuf};

use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows_sys::Win32::Storage::FileSystem::CreateDirectoryW;
use windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW;

/// Full control for Administrators and SYSTEM, inherited by everything
/// inside, and nothing inherited from the parent.
const ADMINS_ONLY: &str = "D:P(A;OICI;FA;;;BA)(A;OICI;FA;;;SY)";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// `C:\Windows\Temp`, asked of Windows rather than read from the
/// environment, which the process that started the broker chose.
fn windows_temp() -> Result<PathBuf, String> {
    let mut buf = [0u16; 260];
    // SAFETY: the buffer's length is passed; the call writes at most that
    // many units and returns how many it wrote.
    let n = unsafe { GetWindowsDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if n == 0 || n >= buf.len() {
        return Err("cannot find the Windows folder".into());
    }
    Ok(PathBuf::from(String::from_utf16_lossy(&buf[..n])).join("Temp"))
}

#[derive(Debug)]
pub struct ProtectedDir {
    path: PathBuf,
}

impl ProtectedDir {
    /// A new folder in the Windows folder's `Temp`.
    pub fn create() -> Result<Self, String> {
        Self::create_in(&windows_temp()?, ADMINS_ONLY)
    }

    /// A new folder in `parent` with the access list `sddl`. Fails if the
    /// name exists, rather than using a folder someone else made.
    fn create_in(parent: &Path, sddl: &str) -> Result<Self, String> {
        let path = parent.join(format!("keyjutsu-broker-{}", &crate::random_hex()[..32]));
        create_with(&path, sddl)?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Make the folder `path` with the owner and access list `sddl`. Fails if it
/// exists, rather than taking over a folder someone else made.
pub fn create_with(path: &Path, sddl: &str) -> Result<(), String> {
    let sddl = wide(sddl);
    let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: `sddl` is NUL-terminated; the descriptor allocated on
    // success is freed below, after the folder has taken its copy.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut sd,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err("cannot build the folder's access list".into());
    }
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd,
        bInheritHandle: 0,
    };
    let name = wide(&path.display().to_string());
    // SAFETY: `name` is NUL-terminated and `sa` points at a valid
    // descriptor for the duration of the call.
    let made = unsafe { CreateDirectoryW(name.as_ptr(), &sa) };
    // SAFETY: allocated by the conversion above; the folder keeps its own copy.
    unsafe { LocalFree(sd) };
    if made == 0 {
        return Err(format!("cannot create {}: {}", path.display(), std::io::Error::last_os_error()));
    }
    Ok(())
}

impl Drop for ProtectedDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("keyjutsu-protected-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap_or_default();
        d
    }

    /// Unelevated, this test cannot be an Administrator, so it checks the
    /// mechanism with an access list naming only the test's own account:
    /// the folder is made with exactly the list given, refuses an existing
    /// name, and goes when dropped.
    #[test]
    fn the_folder_is_made_new_with_the_access_list_given_and_removed_after() {
        let parent = scratch("mechanism");
        let me = crate::pipe::current_user_sid().expect("this account's SID");
        let dir = ProtectedDir::create_in(&parent, &format!("D:P(A;OICI;FA;;;{me})")).expect("created");
        std::fs::write(dir.path().join("x"), "x").expect("the account named can write");
        let acl = keyjutsu_core::terminal::shell::command("icacls").arg(dir.path()).output().expect("icacls");
        let acl = String::from_utf8_lossy(&acl.stdout).to_string();
        assert!(!acl.contains("BUILTIN\\Users") && !acl.contains("Everyone"), "{acl}");
        assert!(!acl.contains("(I)"), "nothing is inherited from the parent: {acl}");
        let path = dir.path().to_owned();
        drop(dir);
        assert!(!path.exists(), "removed when dropped");
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// An access list for Administrators and SYSTEM only: as an unelevated
    /// account, even the one that made it cannot write into it.
    #[test]
    fn an_unelevated_account_cannot_write_into_the_admins_only_folder() {
        if keyjutsu_core::elevation::is_elevated() {
            return; // Elevated, this account is an Administrator: nothing to show.
        }
        let parent = scratch("admins");
        let dir = ProtectedDir::create_in(&parent, ADMINS_ONLY).expect("created");
        let refused = std::fs::write(dir.path().join("planted.exe"), "x");
        // The account that made it still owns it, and an owner may always
        // rewrite the access list: that is how the test cleans up.
        let path = dir.path().to_owned();
        std::mem::forget(dir);
        let _ = keyjutsu_core::terminal::shell::command("icacls").arg(&path).arg("/reset").output();
        let _ = std::fs::remove_dir_all(&parent);
        assert!(refused.is_err(), "an unelevated account wrote into the Administrators' folder");
    }
}
