//! `keyjutsu-broker`: started elevated by KeyJutsu, never by hand.
//!
//! `keyjutsu-broker --pipe NAME --client-pid PID --snapshot FILE
//! --snapshot-hash HASH --secret SECRET`
//!
//! It loads and verifies the snapshot, refuses unless its hash is the one it
//! was launched for, creates its pipe, accepts one connection from the
//! launching process, serves it, and exits. If nobody connects within a
//! minute it exits anyway.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::ExitCode;
use std::time::Duration;

#[cfg(windows)]
fn main() -> ExitCode {
    use keyjutsu_broker::{Broker, accept_launcher, exit, pipe::ServerPipe, run_in_shell, serve};
    use keyjutsu_core::plan::ApprovedSnapshot;

    let args: Vec<String> = std::env::args().collect();
    let arg = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    let (Some(pipe), Some(pid), Some(file), Some(hash), Some(secret)) =
        (arg("--pipe"), arg("--client-pid"), arg("--snapshot"), arg("--snapshot-hash"), arg("--secret"))
    else {
        return ExitCode::from(exit::USAGE);
    };
    let Ok(pid) = pid.parse::<u32>() else { return ExitCode::from(exit::USAGE) };
    let Ok(text) = std::fs::read_to_string(&file) else { return ExitCode::from(exit::SNAPSHOT_UNREADABLE) };
    // The snapshot is verified in full, and must be the one the launch named:
    // a file swapped between approval and elevation is refused.
    let Ok(snapshot) = ApprovedSnapshot::from_json(&text) else {
        return ExitCode::from(exit::SNAPSHOT_NOT_VERIFIED);
    };
    if snapshot.snapshot_hash() != hash {
        return ExitCode::from(exit::SNAPSHOT_NOT_LAUNCHED);
    }
    let Ok(server) = ServerPipe::create(&pipe) else { return ExitCode::from(exit::PIPE) };
    // Nobody connecting within a minute: give up rather than wait elevated.
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(60));
        std::process::exit(i32::from(exit::NOBODY_CAME));
    });
    if accept_launcher(&server, pid).is_err() {
        return ExitCode::from(exit::WRONG_CLIENT);
    }
    let mut broker = Broker::new(snapshot, secret, Box::new(run_in_shell));
    let mut server = server;
    match serve(&mut server, &mut broker) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

#[cfg(not(windows))]
fn main() -> ExitCode {
    ExitCode::FAILURE
}
