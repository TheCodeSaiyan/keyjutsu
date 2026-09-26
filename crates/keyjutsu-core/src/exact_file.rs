//! Reading, writing and removing exactly the file a plan named, and nothing
//! a link leads to.
//!
//! Capture and recovery touch files a plan declares, and for an Administrator
//! step the elevation broker does it as Administrator. An ordinary file call
//! follows symbolic links and junctions, so anything running as the user
//! could swap a folder on the way for a junction between capture and
//! recovery and have the broker write, or delete, somewhere it chose. Here
//! each file is opened without following a final link, and the handle is
//! then asked where it really is: if Windows' final path for it is not the
//! path the plan named (a junction or link on the way), if it is itself a
//! link, or if it has a second name (a hard link), nothing is read, written
//! or removed. Everything after the check goes through the same handle, so
//! nothing can be swapped in between.

use std::io;

/// Why a path is refused: never a Windows error, always a sentence.
fn refused(path: &str, why: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("`{path}` {why}; KeyJutsu touches only the file named"),
    )
}

/// The contents of `path`, or `None` if there is no such file.
pub fn read(path: &str) -> io::Result<Option<Vec<u8>>> {
    imp::read(path)
}

/// Replace the contents of `path` with `bytes`, making it if need be.
pub fn write(path: &str, bytes: &[u8]) -> io::Result<()> {
    imp::write(path, bytes)
}

/// Remove `path`; nothing to do if there is no such file.
pub fn remove(path: &str) -> io::Result<()> {
    imp::remove(path)
}

fn plain(path: &str) -> io::Result<String> {
    keyjutsu_validation::paths::plain_file_path(path)
        .map_err(|why| io::Error::new(io::ErrorKind::InvalidInput, why))?;
    Ok(path.replace('/', "\\"))
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use std::fs::File;
    use std::io::{self, Read, Write};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};

    use windows_sys::Win32::Foundation::{
        ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, GENERIC_READ, GENERIC_WRITE,
        GetLastError, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CreateFileW, DELETE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_DISPOSITION_INFO, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_NAME_NORMALIZED, FILE_SHARE_READ, FileDispositionInfo, GetFileInformationByHandle,
        GetFinalPathNameByHandleW, GetLongPathNameW, OPEN_ALWAYS, OPEN_EXISTING, SetFileInformationByHandle,
        VOLUME_NAME_DOS,
    };

    use super::{plain, refused};

    /// Open `path` without following a final link. `None` if it is not there
    /// and `create` was not asked for; otherwise the file, and whether this
    /// call made it.
    fn open(path: &str, access: u32, create: bool) -> io::Result<Option<(File, bool)>> {
        let wanted = plain(path)?;
        let wide: Vec<u16> = std::ffi::OsStr::new(&wanted).encode_wide().chain(std::iter::once(0)).collect();
        let disposition = if create { OPEN_ALWAYS } else { OPEN_EXISTING };
        // SAFETY: `wide` is NUL-terminated and outlives the call; the other
        // arguments are plain values or null.
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                access,
                FILE_SHARE_READ,
                std::ptr::null(),
                disposition,
                FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
                std::ptr::null_mut(),
            )
        };
        // SAFETY: reads this thread's last error, set by the call above.
        let error = unsafe { GetLastError() };
        if handle == INVALID_HANDLE_VALUE {
            if !create && (error == ERROR_FILE_NOT_FOUND || error == ERROR_PATH_NOT_FOUND) {
                return Ok(None);
            }
            return Err(io::Error::from_raw_os_error(error as i32));
        }
        // SAFETY: `handle` is a valid file handle this function owns; the
        // File closes it.
        let file = unsafe { File::from_raw_handle(handle) };
        let made = create && error != ERROR_ALREADY_EXISTS;
        if let Err(e) = check(&file, &wanted, path) {
            if made {
                // Made where it should not be: taken away again, through
                // the same handle, before anyone sees it.
                let _ = delete(&file);
            }
            return Err(e);
        }
        Ok(Some((file, made)))
    }

    /// The opened file is the named one: not a link or folder, one name
    /// only, and really at the path asked for.
    fn check(file: &File, wanted: &str, path: &str) -> io::Result<()> {
        // SAFETY: a plain C structure of integers, for which all zeroes is valid.
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: the handle is valid for the File's life; `info` is a
        // correctly sized buffer.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(refused(path, "is a link"));
        }
        if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
            return Err(refused(path, "is a folder"));
        }
        if info.nNumberOfLinks > 1 {
            return Err(refused(path, "has another name elsewhere (a hard link)"));
        }
        let mut buf = vec![0u16; 1024];
        loop {
            // SAFETY: `buf` has room for `buf.len()` UTF-16 units.
            let n = unsafe {
                GetFinalPathNameByHandleW(
                    file.as_raw_handle(),
                    buf.as_mut_ptr(),
                    buf.len() as u32,
                    FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
                )
            } as usize;
            if n == 0 {
                return Err(io::Error::last_os_error());
            }
            if n < buf.len() {
                buf.truncate(n);
                break;
            }
            buf.resize(n + 1, 0);
        }
        let real = String::from_utf16_lossy(&buf);
        let real = real.strip_prefix(r"\\?\").unwrap_or(&real);
        // Short names (RUNNER~1) are the same folder, so the path asked for
        // is compared in its long form. That form keeps a junction's own
        // name, so a junction on the way still does not match.
        let wanted = long_form(wanted);
        if real.to_lowercase() != wanted.to_lowercase() {
            return Err(refused(path, &format!("leads to {real}, through a link or junction on the way")));
        }
        Ok(())
    }

    /// `path` with every short (8.3) name spelt out; as given if Windows
    /// cannot say.
    fn long_form(path: &str) -> String {
        let wide: Vec<u16> = std::ffi::OsStr::new(path).encode_wide().chain(std::iter::once(0)).collect();
        let mut buf = vec![0u16; 1024];
        loop {
            // SAFETY: `wide` is NUL-terminated; `buf` has room for its length.
            let n = unsafe { GetLongPathNameW(wide.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
            if n == 0 {
                return path.to_owned();
            }
            if n < buf.len() {
                return String::from_utf16_lossy(&buf[..n]);
            }
            buf.resize(n + 1, 0);
        }
    }

    fn delete(file: &File) -> io::Result<()> {
        let info = FILE_DISPOSITION_INFO { DeleteFile: true };
        // SAFETY: the handle was opened with DELETE access; `info` is the
        // structure this information class takes, with its exact size.
        let ok = unsafe {
            SetFileInformationByHandle(
                file.as_raw_handle(),
                FileDispositionInfo,
                (&info as *const FILE_DISPOSITION_INFO).cast(),
                std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
            )
        };
        if ok == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }

    pub fn read(path: &str) -> io::Result<Option<Vec<u8>>> {
        let Some((mut file, _)) = open(path, GENERIC_READ, false)? else { return Ok(None) };
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(Some(bytes))
    }

    pub fn write(path: &str, bytes: &[u8]) -> io::Result<()> {
        let Some((mut file, _)) = open(path, GENERIC_READ | GENERIC_WRITE | DELETE, true)? else {
            return Err(refused(path, "could not be opened"));
        };
        // Emptied only now, after the check: never before it.
        file.set_len(0)?;
        file.write_all(bytes)?;
        file.flush()
    }

    pub fn remove(path: &str) -> io::Result<()> {
        match open(path, GENERIC_READ | DELETE, false)? {
            None => Ok(()),
            Some((file, _)) => delete(&file),
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::io;

    use super::plain;

    pub fn read(path: &str) -> io::Result<Option<Vec<u8>>> {
        match std::fs::read(plain(path)?) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn write(path: &str, bytes: &[u8]) -> io::Result<()> {
        std::fs::write(plain(path)?, bytes)
    }

    pub fn remove(path: &str) -> io::Result<()> {
        match std::fs::remove_file(plain(path)?) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}
