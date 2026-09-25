//! The broker's end of its named pipe (ADR 0011). The pipe admits only the
//! Windows account that started the broker (and SYSTEM), rejects remote
//! clients, refuses to be created if the name is already taken, and reports
//! which process connected so the broker can check it is the one that
//! launched it. These are the only `unsafe` calls in this crate.

#![allow(unsafe_code)]

use std::io::{Read, Write};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_PIPE_CONNECTED, GetLastError, HANDLE, INVALID_HANDLE_VALUE, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX, ReadFile, WriteFile,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The string SID of the account this process runs as.
pub fn current_user_sid() -> Result<String, String> {
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: a pseudo-handle for this process; the token handle written on
    // success is closed below.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err("cannot open this process's token".into());
    }
    let mut len = 0u32;
    // SAFETY: a size query with no buffer; it only writes `len`.
    unsafe { GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut len) };
    let mut buf = vec![0u8; len as usize];
    // SAFETY: `buf` is `len` bytes, as the query asked for.
    let ok = unsafe { GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len) };
    // SAFETY: the token handle is not used after this.
    unsafe { CloseHandle(token) };
    if ok == 0 || buf.len() < std::mem::size_of::<TOKEN_USER>() {
        return Err("cannot read this process's user".into());
    }
    // SAFETY: the buffer holds a TOKEN_USER written by the call above; it is
    // read unaligned because a Vec<u8> promises no alignment.
    let user: TOKEN_USER = unsafe { std::ptr::read_unaligned(buf.as_ptr().cast()) };
    let mut text: *mut u16 = std::ptr::null_mut();
    // SAFETY: the SID pointer points into `buf`, which is alive; the string
    // DPAPI-style allocated with LocalAlloc is freed below.
    if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut text) } == 0 {
        return Err("cannot format the user's SID".into());
    }
    // SAFETY: `text` is a NUL-terminated wide string from the call above.
    let sid = unsafe {
        let mut n = 0;
        while *text.add(n) != 0 {
            n += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(text, n))
    };
    // SAFETY: allocated by ConvertSidToStringSidW with LocalAlloc; not used after.
    unsafe { LocalFree(text.cast()) };
    Ok(sid)
}

/// One named pipe, one client.
#[derive(Debug)]
pub struct ServerPipe {
    handle: HANDLE,
}

// SAFETY: the handle is used by one thread at a time (the broker serves one
// client sequentially); Windows pipe handles may be used from any thread.
unsafe impl Send for ServerPipe {}

impl ServerPipe {
    /// Create `\\.\pipe\<name>`, admitting only this account and SYSTEM.
    pub fn create(name: &str) -> Result<Self, String> {
        let sid = current_user_sid()?;
        // Protected DACL: full access for this user and SYSTEM, nobody else.
        let sddl = wide(&format!("D:P(A;;GA;;;{sid})(A;;GA;;;SY)"));
        let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: `sddl` is NUL-terminated; the descriptor allocated on
        // success is freed below after the pipe has copied it.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut sd,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err("cannot build the pipe's access list".into());
        }
        let sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd,
            bInheritHandle: 0,
        };
        let path = wide(&format!(r"\\.\pipe\{name}"));
        // SAFETY: `path` is NUL-terminated and `sa` points at a valid
        // descriptor for the duration of the call.
        let handle = unsafe {
            CreateNamedPipeW(
                path.as_ptr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                65_536,
                65_536,
                0,
                &sa,
            )
        };
        // SAFETY: allocated by the conversion above; the pipe keeps its own copy.
        unsafe { LocalFree(sd) };
        if handle == INVALID_HANDLE_VALUE {
            return Err(format!(
                "cannot create the broker's pipe (it may already exist): error {}",
                unsafe {
                    // SAFETY: reads this thread's last error; no preconditions.
                    GetLastError()
                }
            ));
        }
        Ok(Self { handle })
    }

    /// Wait for a client and return its process id.
    pub fn accept(&self) -> Result<u32, String> {
        // SAFETY: a valid pipe handle; no overlapped structure (blocking).
        let ok = unsafe { ConnectNamedPipe(self.handle, std::ptr::null_mut()) };
        // SAFETY: reads this thread's last error; no preconditions.
        if ok == 0 && unsafe { GetLastError() } != ERROR_PIPE_CONNECTED {
            return Err("no client connected".into());
        }
        let mut pid = 0u32;
        // SAFETY: a connected pipe handle; writes the client's pid.
        if unsafe { GetNamedPipeClientProcessId(self.handle, &mut pid) } == 0 {
            return Err("cannot tell which process connected".into());
        }
        Ok(pid)
    }
}

impl Read for ServerPipe {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut n = 0u32;
        let len = u32::try_from(buf.len()).unwrap_or(u32::MAX);
        // SAFETY: `buf` is valid for `len` bytes; no overlapped I/O.
        let ok = unsafe { ReadFile(self.handle, buf.as_mut_ptr(), len, &mut n, std::ptr::null_mut()) };
        if ok == 0 {
            // A client that has gone reads as the end of the stream.
            return Ok(0);
        }
        Ok(n as usize)
    }
}

impl Write for ServerPipe {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut n = 0u32;
        let len = u32::try_from(buf.len()).unwrap_or(u32::MAX);
        // SAFETY: `buf` is valid for `len` bytes; no overlapped I/O.
        let ok = unsafe { WriteFile(self.handle, buf.as_ptr(), len, &mut n, std::ptr::null_mut()) };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(n as usize)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for ServerPipe {
    fn drop(&mut self) {
        // SAFETY: the handle is owned by this value and not used after drop.
        unsafe {
            DisconnectNamedPipe(self.handle);
            CloseHandle(self.handle);
        }
    }
}
