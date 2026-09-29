//! Running a helper process with a time limit, and the base64 the PowerShell
//! helpers speak.

use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finished {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RunError {
    #[error("could not start {program}: {detail}")]
    Start { program: String, detail: String },
    #[error("{program} did not finish within {seconds} seconds and was stopped")]
    TimedOut { program: String, seconds: u64 },
    /// Whoever started it asked for it to stop.
    #[error("{program} was stopped")]
    Stopped { program: String },
}

/// Run `command`, feed it `stdin`, and give up after `limit`. Output is read
/// on its own threads so a chatty child cannot fill a pipe and hang.
/// How every program here is made: with no console window (see
/// [keyjutsu_terminal::shell::command]).
pub use keyjutsu_terminal::shell::command;

pub fn run(command: Command, stdin: &str, limit: Duration) -> Result<Finished, RunError> {
    run_stoppable(command, stdin, limit, &AtomicBool::new(false))
}

/// [`run`], which also gives up as soon as `stop` is set, from another thread:
/// the operator no longer wants the answer.
pub fn run_stoppable(
    mut command: Command,
    stdin: &str,
    limit: Duration,
    stop: &AtomicBool,
) -> Result<Finished, RunError> {
    // Never a console window of its own: from the desktop app, which has
    // none, each program started would otherwise flash one up.
    keyjutsu_terminal::shell::no_window(&mut command);
    let program = format!("{:?}", command.get_program());
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| RunError::Start { program: program.clone(), detail: e.to_string() })?;
    if let Some(mut input) = child.stdin.take() {
        let _ = input.write_all(stdin.as_bytes());
    }
    let mut out = child.stdout.take();
    let mut err = child.stderr.take();
    let out_thread = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(o) = out.as_mut() {
            let _ = o.read_to_string(&mut s);
        }
        s
    });
    let err_thread = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(e) = err.as_mut() {
            let _ = e.read_to_string(&mut s);
        }
        s
    });
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if stop.load(Ordering::SeqCst) => {
                end_tree(&mut child);
                return Err(RunError::Stopped { program });
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                end_tree(&mut child);
                return Err(RunError::TimedOut { program, seconds: limit.as_secs() });
            }
        }
    };
    Ok(Finished {
        success: status.success(),
        stdout: out_thread.join().unwrap_or_default(),
        stderr: err_thread.join().unwrap_or_default(),
    })
}

/// End `child` and everything it started. An agent reached through a `.cmd`
/// shim is `cmd.exe` running `node.exe`: ending only the first would leave the
/// agent working, and holding the output pipes open.
fn end_tree(child: &mut Child) {
    if cfg!(windows) {
        let mut taskkill = command("taskkill");
        taskkill.args(["/T", "/F", "/PID", &child.id().to_string()]);
        keyjutsu_terminal::shell::no_window(&mut taskkill);
        let _ = taskkill.stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (i, shift) in [18u32, 12, 6, 0].into_iter().enumerate() {
            out.push(if i <= chunk.len() { TABLE[((n >> shift) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let clean: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if !clean.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(clean.len() / 4 * 3);
    for chunk in clean.chunks(4) {
        let mut n = 0u32;
        let mut pad = 0;
        for &c in chunk {
            let v = match c {
                b'=' => {
                    pad += 1;
                    0
                }
                _ if pad > 0 => return None,
                _ => TABLE.iter().position(|&t| t == c)? as u32,
            };
            n = (n << 6) | v;
        }
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&bytes[..3 - pad]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips_and_matches_rfc_4648() {
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        for s in ["", "f", "fo", "foo", "foob", "fooba", "foobar", "café ✓"] {
            assert_eq!(base64_decode(&base64_encode(s.as_bytes())).unwrap(), s.as_bytes());
        }
        assert!(base64_decode("Zm9").is_none());
        assert!(base64_decode("Zm=v").is_none());
        assert!(base64_decode("Zm9!").is_none());
    }

    #[test]
    fn a_process_that_overruns_is_stopped() {
        let mut c = keyjutsu_terminal::shell::command("cmd");
        c.args(["/D", "/C", "ping -n 30 127.0.0.1 >NUL"]);
        let started = Instant::now();
        let r = run(c, "", Duration::from_millis(500));
        assert!(matches!(r, Err(RunError::TimedOut { .. })), "{r:?}");
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_process_asked_to_stop_stops_with_what_it_started() {
        // The child starts a second cmd that would write a file after two
        // seconds: stopping the child has to stop that one too.
        let marker = std::env::temp_dir().join(format!("kj-stop-{}.txt", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let mut c = keyjutsu_terminal::shell::command("cmd");
        // Raw: cmd reads its own quotes, not the C runtime's escaped ones.
        std::os::windows::process::CommandExt::raw_arg(
            &mut c,
            format!("/D /C cmd /D /C \"ping -n 3 127.0.0.1 >NUL & echo x> {}\"", marker.display()),
        );
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let setter = stop.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            setter.store(true, Ordering::SeqCst);
        });
        let started = Instant::now();
        let r = run_stoppable(c, "", Duration::from_secs(60), &stop);
        assert!(matches!(r, Err(RunError::Stopped { .. })), "{r:?}");
        assert!(started.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_secs(4));
        assert!(!marker.exists(), "what the stopped process started carried on");
    }

    #[test]
    fn output_and_status_come_back() {
        let mut c = keyjutsu_terminal::shell::command("cmd");
        c.args(["/D", "/C", "echo hello& exit 3"]);
        let r = run(c, "", Duration::from_secs(10)).unwrap();
        assert!(!r.success);
        assert_eq!(r.stdout.trim(), "hello");
    }
}
