//! What a file path a plan names for KeyJutsu itself to touch may look like.
//!
//! A capture target is read before a step and written or removed on
//! recovery, by the elevation broker for an Administrator step. It must say
//! plainly which file it means: a whole path on a drive, with nothing for
//! Windows to reinterpret on the way.

/// Whether `target` is a plain absolute file path such as `C:\Tools\a.json`
/// (forward slashes allowed): a drive letter, and then names only. Refused:
/// relative paths, `.` and `..`, UNC shares and device paths (`\\server`,
/// `\\?\`, `\\.\`), alternate data streams (`a.txt:secret`), characters
/// Windows does not allow in names, and names ending in a dot or space,
/// which Windows silently drops.
pub fn plain_file_path(target: &str) -> Result<(), String> {
    let bad = |why: &str| {
        Err(format!("`{target}` {why}; name a file as a whole path, such as C:\\Tools\\settings.json"))
    };
    let b = target.as_bytes();
    if b.len() < 4 || !b[0].is_ascii_alphabetic() || b[1] != b':' || !matches!(b[2], b'\\' | b'/') {
        return bad("is not a whole path on a drive");
    }
    let rest = &target[3..];
    for name in rest.split(['\\', '/']) {
        if name.is_empty() {
            return bad("has an empty folder name");
        }
        if name == "." || name == ".." {
            return bad("steps through `.` or `..`");
        }
        if name.ends_with('.') || name.ends_with(' ') {
            return bad("has a name ending in a dot or a space, which Windows drops");
        }
        if let Some(c) =
            name.chars().find(|c| matches!(c, ':' | '<' | '>' | '"' | '|' | '?' | '*') || c.is_control())
        {
            return bad(&format!("has `{}` in a name", c.escape_default()));
        }
    }
    Ok(())
}

/// Why a step's working directory cannot be entered safely, if it cannot.
/// KeyJutsu types the line that goes there itself: for cmd, `cd /d "DIR"`,
/// where a `"` would end the quotes and let the rest run as a command, and
/// a `%` would be expanded; for any shell, a control character such as a
/// line break would submit what follows it. Windows allows none of `"` or
/// control characters in a folder name, so nothing real is refused.
pub fn working_directory_problem(dir: &str, cmd: bool) -> Option<String> {
    if let Some(c) = dir.chars().find(|c| *c == '"' || c.is_control()) {
        return Some(format!(
            "the working directory has `{}` in it, which no folder name can",
            c.escape_default()
        ));
    }
    (cmd && dir.contains('%'))
        .then(|| "the working directory has `%` in it, which cmd would expand rather than use".to_owned())
}

#[cfg(test)]
mod tests {
    use super::{plain_file_path, working_directory_problem};

    #[test]
    fn a_working_directory_cannot_carry_a_command_in() {
        assert_eq!(working_directory_problem(r"C:\Program Files\App", true), None);
        assert_eq!(working_directory_problem(r"C:\100% done", false), None);
        assert!(working_directory_problem(r#"C:\x" & calc & ""#, true).is_some());
        assert!(working_directory_problem(r#"C:\x" & calc & ""#, false).is_some());
        assert!(working_directory_problem("C:\\x\r\nStop-Computer", false).is_some());
        assert!(working_directory_problem(r"C:\%PATH%", true).is_some());
    }

    #[test]
    fn a_whole_path_on_a_drive_is_plain() {
        for ok in [r"C:\Tools\settings.json", "D:/data/app/config.ini", r"c:\a b\c-d_e.txt"] {
            assert_eq!(plain_file_path(ok), Ok(()), "{ok}");
        }
    }

    #[test]
    fn anything_windows_might_reinterpret_is_refused() {
        for bad in [
            "settings.json",
            r"Tools\settings.json",
            r"\Tools\settings.json",
            r"C:settings.json",
            r"C:\Tools\..\Windows\win.ini",
            r"C:\Tools\.\a.txt",
            r"\\server\share\a.txt",
            r"\\?\C:\Windows\win.ini",
            r"\\.\PhysicalDrive0",
            r"C:\Tools\a.txt:hidden",
            r"C:\Tools\a.txt.",
            r"C:\Tools\a.txt ",
            r"C:\Tools\\a.txt",
            r"C:\Tools\a*.txt",
            "C:\\Tools\\a\u{0}.txt",
        ] {
            assert!(plain_file_path(bad).is_err(), "{bad} should be refused");
        }
    }
}
