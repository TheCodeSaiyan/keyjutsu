//! Code a command line runs inside another: `pwsh -Command`, PowerShell's
//! `-EncodedCommand`, `cmd /c`, `Invoke-Expression` and `Start-Process`.
//!
//! A work profile's deny list would mean little if wrapping a command in
//! another shell got it past. So each of these is unpacked, statically,
//! into the code it would run, which is then parsed and matched like any
//! other line. Nothing is run to do it. What cannot be unpacked without
//! running something (arguments computed when the line runs, a script file
//! handed to a shell, a pipeline feeding `Invoke-Expression`) is reported as
//! opaque, and a profile that refuses opaque code fails the step.

/// What a command runs inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inner {
    /// PowerShell code, to parse like a line of its own.
    PowerShell(String),
    /// A cmd.exe command line.
    Cmd(String),
    /// A program started with these arguments.
    Program(String, Vec<String>),
    /// Code KeyJutsu cannot see without running something, and why.
    Opaque(String),
}

/// What `name` (an identity, as [`crate::profile::identity`] gives it) runs
/// inside it when given `arguments`, if anything.
pub fn inner(name: &str, arguments: &[String], static_arguments: bool, from_pipeline: bool) -> Option<Inner> {
    let computed =
        || Inner::Opaque(format!("`{name}` runs code given in arguments computed when the line runs"));
    match name {
        "pwsh" | "powershell" => {
            if arguments.is_empty() {
                return None;
            }
            if !static_arguments {
                return Some(computed());
            }
            Some(powershell_cli(name, arguments))
        }
        "cmd" => {
            let i = arguments.iter().position(|a| {
                let a = a.to_ascii_lowercase();
                a == "/c" || a == "/k" || a == "/r"
            })?;
            if !static_arguments {
                return Some(computed());
            }
            Some(Inner::Cmd(join(&arguments[i + 1..])))
        }
        "invoke-expression" | "iex" => {
            if from_pipeline {
                return Some(Inner::Opaque(format!("`{name}` runs whatever the pipeline before it yields")));
            }
            if !static_arguments {
                return Some(computed());
            }
            let values: Vec<&String> =
                arguments.iter().filter(|a| !a.to_ascii_lowercase().starts_with("-c")).collect();
            match values.as_slice() {
                [code] => Some(Inner::PowerShell(crate::profile::unquote(code))),
                _ => Some(Inner::Opaque(format!("`{name}` is not given its code as one literal string"))),
            }
        }
        "start-process" | "saps" | "start" => {
            if !static_arguments {
                return Some(computed());
            }
            start_process(arguments)
        }
        _ => None,
    }
}

/// The command-line options of `pwsh.exe` and `powershell.exe`, as far as
/// they decide what runs. Anything unrecognised is opaque: guessing how the
/// shell would read it is how a deny list gets around.
fn powershell_cli(name: &str, arguments: &[String]) -> Inner {
    // Options that take a value, and those that take none, by their
    // shortest accepted forms.
    const WITH_VALUE: &[(&str, usize)] = &[
        ("executionpolicy", 2),
        ("ep", 2),
        ("windowstyle", 1),
        ("inputformat", 2),
        ("outputformat", 1),
        ("version", 1),
        ("workingdirectory", 2),
        ("wd", 2),
        ("configurationname", 3),
    ];
    const SWITCHES: &[(&str, usize)] = &[
        ("noprofile", 3),
        ("nologo", 3),
        ("noninteractive", 4),
        ("noexit", 3),
        ("sta", 3),
        ("mta", 3),
        ("login", 1),
    ];
    let abbreviates =
        |given: &str, (full, min): &(&str, usize)| given.len() >= *min && full.starts_with(given);
    let mut i = 0;
    while i < arguments.len() {
        let a = crate::profile::unquote(&arguments[i]);
        let Some(option) = a.strip_prefix('-').or_else(|| a.strip_prefix('/')) else {
            // The first bare argument: Windows PowerShell reads it as a
            // command, PowerShell 7 as a script file.
            return if name == "powershell" {
                Inner::PowerShell(join(&arguments[i..]))
            } else {
                Inner::Opaque(format!("`{name}` is handed the script file {a}, which KeyJutsu does not read"))
            };
        };
        let option = option.to_ascii_lowercase();
        if option == "ec" || abbreviates(&option, &("encodedcommand", 1)) {
            return match arguments
                .get(i + 1)
                .map(|v| crate::profile::unquote(v))
                .and_then(|v| decode_encoded(&v))
            {
                Some(code) => Inner::PowerShell(code),
                None => Inner::Opaque(format!("`{name} -{option}` is not given valid Base64 UTF-16 text")),
            };
        }
        if abbreviates(&option, &("command", 1)) {
            let rest = &arguments[i + 1..];
            return match rest {
                [] => Inner::Opaque(format!("`{name} -{option}` is given no command")),
                [only] if crate::profile::unquote(only) == "-" => {
                    Inner::Opaque(format!("`{name}` reads its commands from what is piped to it"))
                }
                _ => Inner::PowerShell(join(rest)),
            };
        }
        if abbreviates(&option, &("file", 1)) {
            let file = arguments.get(i + 1).map(|f| crate::profile::unquote(f)).unwrap_or_default();
            return Inner::Opaque(format!(
                "`{name}` is handed the script file {file}, which KeyJutsu does not read"
            ));
        }
        if WITH_VALUE.iter().any(|w| abbreviates(&option, w)) {
            i += 2;
        } else if SWITCHES.iter().any(|s| abbreviates(&option, s)) {
            i += 1;
        } else {
            return Inner::Opaque(format!(
                "`{name}` is given -{option}, which KeyJutsu does not know how the shell reads"
            ));
        }
    }
    // Only options: an interactive shell that runs nothing of the plan's.
    Inner::Opaque(format!("`{name}` is started with no command, as an interactive shell"))
}

fn start_process(arguments: &[String]) -> Option<Inner> {
    let mut file: Option<String> = None;
    let mut args: Vec<String> = Vec::new();
    let mut positional = 0;
    let mut i = 0;
    while i < arguments.len() {
        let a = &arguments[i];
        let lower = a.to_ascii_lowercase();
        if let Some(p) = lower.strip_prefix('-') {
            let value = arguments.get(i + 1).cloned();
            if "filepath".starts_with(p) || p == "path" || p == "pspath" {
                file = value;
            } else if "argumentlist".starts_with(p) || p == "args" {
                args = value.map(|v| split_list(&v)).unwrap_or_default();
            } else if ["nonewwindow", "wait", "passthru", "usenewenvironment", "loaduserprofile"]
                .iter()
                .any(|s| s.starts_with(p))
            {
                i += 1;
                continue;
            }
            i += 2;
            continue;
        }
        match positional {
            0 => file = Some(a.clone()),
            1 => args = split_list(a),
            _ => {}
        }
        positional += 1;
        i += 1;
    }
    let file = crate::profile::unquote(&file?);
    Some(Inner::Program(file, args))
}

/// A PowerShell list or string of arguments, as separate arguments.
fn split_list(value: &str) -> Vec<String> {
    let parts: Vec<String> = value.split(',').map(|p| crate::profile::unquote(p.trim())).collect();
    if parts.len() == 1 { tokens(&parts[0]) } else { parts }
}

/// Arguments joined back into one line, a lone quoted argument unquoted.
fn join(arguments: &[String]) -> String {
    match arguments {
        [only] => crate::profile::unquote(only),
        _ => arguments.join(" "),
    }
}

/// A cmd.exe line as the commands it runs: each segment between `&`, `&&`,
/// `|` and `||` outside quotes, split into words with quotes kept together.
pub fn cmd_segments(line: &str) -> Vec<(String, Vec<String>)> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for ch in line.chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                current.push(ch);
            }
            '&' | '|' if !quoted => segments.push(std::mem::take(&mut current)),
            _ => current.push(ch),
        }
    }
    segments.push(current);
    segments
        .iter()
        .filter_map(|s| {
            let mut words = tokens(s.trim_start_matches('@').trim()).into_iter();
            let first = words.next()?;
            Some((first, words.collect()))
        })
        .collect()
}

/// Words separated by whitespace, with double-quoted runs kept whole and
/// their quotes kept, as PowerShell's parser reports arguments.
fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for ch in text.chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                current.push(ch);
            }
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// What `-EncodedCommand` carries: Base64 of UTF-16LE text.
pub fn decode_encoded(text: &str) -> Option<String> {
    let bytes = base64(text)?;
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    String::from_utf16(&units).ok()
}

fn base64(text: &str) -> Option<Vec<u8>> {
    let value = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let clean: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let body = clean.strip_suffix(b"==").or_else(|| clean.strip_suffix(b"=")).unwrap_or(&clean);
    if body.is_empty() || !clean.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(body.len() * 3 / 4);
    for chunk in body.chunks(4) {
        let mut n = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            n |= value(*c)? << (18 - 6 * i);
        }
        let bytes = n.to_be_bytes();
        out.extend_from_slice(&bytes[1..chunk.len()]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// `[Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($s))`.
    fn encode(s: &str) -> String {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let bytes: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | (*b as u32) << (16 - 8 * i));
            for i in 0..4 {
                if i <= chunk.len() {
                    out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    #[test]
    fn encoded_commands_are_decoded() {
        for code in ["Format-Volume -DriveLetter D", "a", "ab", "Clear-EventLog Security"] {
            assert_eq!(decode_encoded(&encode(code)).as_deref(), Some(code));
            assert_eq!(
                inner("pwsh", &args(&["-NoProfile", "-enc", &encode(code)]), true, false),
                Some(Inner::PowerShell(code.into()))
            );
        }
        assert_eq!(
            inner("powershell", &args(&["-EncodedCommand", "not base64!"]), true, false),
            Some(Inner::Opaque("`powershell -encodedcommand` is not given valid Base64 UTF-16 text".into()))
        );
    }

    #[test]
    fn shells_are_unpacked_and_unknown_options_are_opaque() {
        assert_eq!(
            inner(
                "pwsh",
                &args(&["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", "'Clear-Disk 1'"]),
                true,
                false
            ),
            Some(Inner::PowerShell("Clear-Disk 1".into()))
        );
        assert_eq!(
            inner("powershell", &args(&["-nop", "Remove-Item", "x"]), true, false),
            Some(Inner::PowerShell("Remove-Item x".into()))
        );
        assert!(matches!(inner("pwsh", &args(&["-File", "x.ps1"]), true, false), Some(Inner::Opaque(_))));
        assert!(matches!(inner("pwsh", &args(&["x.ps1"]), true, false), Some(Inner::Opaque(_))));
        assert!(matches!(inner("pwsh", &args(&["-Mystery", "x"]), true, false), Some(Inner::Opaque(_))));
        assert!(matches!(inner("pwsh", &args(&["-c", "$x"]), false, false), Some(Inner::Opaque(_))));
        assert_eq!(
            inner("cmd", &args(&["/d", "/c", "\"format d: /q\""]), true, false),
            Some(Inner::Cmd("format d: /q".into()))
        );
        assert_eq!(inner("cmd", &args(&["/?"]), true, false), None);
    }

    #[test]
    fn invoke_expression_needs_one_literal_string() {
        assert_eq!(
            inner("invoke-expression", &args(&["'Format-Volume D'"]), true, false),
            Some(Inner::PowerShell("Format-Volume D".into()))
        );
        assert_eq!(
            inner("iex", &args(&["-Command", "\"Stop-Computer\""]), true, false),
            Some(Inner::PowerShell("Stop-Computer".into()))
        );
        assert!(matches!(inner("iex", &args(&[]), true, true), Some(Inner::Opaque(_))));
        assert!(matches!(inner("iex", &args(&["$code"]), false, false), Some(Inner::Opaque(_))));
    }

    #[test]
    fn start_process_names_the_program_it_starts() {
        assert_eq!(
            inner(
                "start-process",
                &args(&["-FilePath", "'diskpart.exe'", "-ArgumentList", "'/s x.txt'"]),
                true,
                false
            ),
            Some(Inner::Program("diskpart.exe".into(), args(&["/s", "x.txt"])))
        );
        assert_eq!(
            inner("start-process", &args(&["pwsh", "'-c','Clear-Disk 1'"]), true, false),
            Some(Inner::Program("pwsh".into(), args(&["-c", "Clear-Disk 1"])))
        );
        assert!(matches!(inner("start-process", &args(&["$exe"]), false, false), Some(Inner::Opaque(_))));
    }

    #[test]
    fn cmd_lines_split_into_commands_outside_quotes() {
        let segments = cmd_segments("@echo off && format d: /q | find \"a&b\"");
        let names: Vec<&str> = segments.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["echo", "format", "find"]);
        assert_eq!(segments[2].1, ["\"a&b\""]);
    }
}
