//! An update runs only an installer that matches its release's checksums
//! and is signed by KeyJutsu's publisher, and never while a run is changing
//! the machine. Real files, a real signature check, and a small HTTP server
//! on 127.0.0.1 standing in for GitHub.

#![cfg(windows)]
#![allow(clippy::unwrap_used)] // Helpers outside #[test] functions may unwrap too.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};

use keyjutsu_core::plan::hash::sha256_hex;
use keyjutsu_core::runlock::RunLock;
use keyjutsu_core::update::{Channel, download, free_to_install, pick, start, verify};
use serde_json::json;

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("update").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Serves each path its own body.
fn serve(files: HashMap<&'static str, Vec<u8>>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let mut buf = [0u8; 4096];
            let mut seen = Vec::new();
            while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
                match s.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => seen.extend_from_slice(&buf[..n]),
                }
            }
            let request = String::from_utf8_lossy(&seen);
            let path = request.split_whitespace().nth(1).unwrap_or("/");
            let body = files.get(path).cloned().unwrap_or_default();
            let head =
                format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let _ = s.write_all(head.as_bytes());
            let _ = s.write_all(&body);
        }
    });
    port
}

const NAME: &str = "KeyJutsu_9.9.9_x64-setup.exe";

fn sums_for(bytes: &[u8]) -> String {
    format!("{}  {NAME}\n", sha256_hex(bytes))
}

#[test]
fn an_installer_that_is_unsigned_altered_or_signed_by_someone_else_is_refused() {
    let dir = scratch("verify");

    // Not a signed program (Windows reports it NotSigned, or UnknownError for a file that is not a program at all), though its checksum matches.
    let unsigned = dir.join(NAME);
    std::fs::write(&unsigned, b"MZ not really an installer").unwrap();
    let why = verify(&unsigned, &sums_for(b"MZ not really an installer"), NAME).unwrap_err();
    assert!(why.contains("does not verify") && why.contains("must be signed"), "{why}");

    // Altered after the checksums were published.
    let why = verify(&unsigned, &sums_for(b"what was published"), NAME).unwrap_err();
    assert!(why.contains("does not match SHA256SUMS"), "{why}");

    // Not listed at all.
    let why = verify(&unsigned, "", NAME).unwrap_err();
    assert!(why.contains("does not list"), "{why}");

    // Validly signed, by Microsoft: a valid signature is not enough.
    let signed = dir.join("signed").join(NAME);
    std::fs::create_dir_all(signed.parent().unwrap()).unwrap();
    std::fs::copy(r"C:\Windows\System32\notepad.exe", &signed).unwrap();
    let bytes = std::fs::read(&signed).unwrap();
    let why = verify(&signed, &sums_for(&bytes), NAME).unwrap_err();
    assert!(why.contains("not by TheCodeSaiyan Ltd"), "{why}");
}

#[test]
fn a_download_that_fails_its_checks_is_deleted_not_kept() {
    let installer = b"MZ unsigned stand-in".to_vec();
    let port = serve(HashMap::from([
        ("/setup.exe", installer.clone()),
        ("/SHA256SUMS", sums_for(&installer).into_bytes()),
    ]));
    let releases = json!([{
        "tag_name": "v9.9.9", "prerelease": false, "draft": false, "body": "",
        "assets": [
            {"name": NAME, "browser_download_url": format!("http://127.0.0.1:{port}/setup.exe")},
            {"name": "SHA256SUMS", "browser_download_url": format!("http://127.0.0.1:{port}/SHA256SUMS")}
        ]
    }]);
    let available = pick(&releases, Channel::Stable, "0.1.0").unwrap();
    let dir = scratch("download");
    let why = download(&available, &dir).unwrap_err();
    assert!(why.contains("does not verify") && why.contains("nothing was installed"), "{why}");
    assert!(!dir.join(NAME).exists() && !dir.join("SHA256SUMS").exists(), "nothing was kept");
}

/// Uses the machine's own run lock, as the app and CLI do: while a run holds
/// it, no update starts, and nothing is run.
#[test]
fn an_update_waits_while_a_run_changes_the_machine() {
    let dir = scratch("waits");
    let marker = dir.join("ran.txt");
    let installer = dir.join("stand-in.cmd");
    std::fs::write(&installer, format!("@echo ran> \"{}\"\r\n", marker.display())).unwrap();
    let running = RunLock::take().unwrap();
    let why = free_to_install().unwrap_err();
    assert!(why.contains("another KeyJutsu run") && why.contains("waits"), "{why}");
    assert!(start(&installer).is_err());
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert!(!marker.exists(), "nothing was started");
    drop(running);
    assert!(free_to_install().is_ok(), "free once the run ends");
}
