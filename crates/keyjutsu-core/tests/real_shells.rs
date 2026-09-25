//! Real shells through a real pseudo-console. Nothing here is mocked: each
//! test starts PowerShell 7, Windows PowerShell 5.1 or cmd.exe in ConPTY and
//! reads back what the shell actually did.
//!
//! The clean profile is used so a developer's own profile cannot change the
//! outcome; the readiness probe covers the detected profile.

#![allow(clippy::unwrap_used)] // Helpers outside #[test] functions may unwrap too.
#![cfg(windows)]

use std::sync::Arc;
use std::time::Duration;

use keyjutsu_core::execution::{
    ExecutionMode, ExecutionState, PerformanceConfig, StagedScript, StagedStep, StepOutcome,
};
use keyjutsu_core::headless::Collector;
use keyjutsu_core::terminal::{KeyChord, KeyName, ProfileMode, ShellKind, TerminalSize};
use keyjutsu_core::{CoreError, Session, SessionEvent, SessionOptions};

const TIMEOUT: Duration = Duration::from_secs(30);

fn start(kind: ShellKind) -> (Session, Arc<Collector>) {
    let collector = Arc::new(Collector::new());
    let mut options = SessionOptions::new(kind);
    options.profile = ProfileMode::Clean;
    options.intercept_cursor_queries = true;
    let session = Session::open(options, collector.clone()).expect("shell starts");
    assert!(
        session.wait_ready(TIMEOUT),
        "{kind:?} never drew a KeyJutsu prompt:\n{}",
        collector.plain_output()
    );
    (session, collector)
}

fn script(steps: &[(&str, Option<ExecutionMode>)]) -> StagedScript {
    StagedScript {
        steps: steps
            .iter()
            .enumerate()
            .map(|(i, (command, mode))| StagedStep {
                id: format!("s{i}"),
                title: format!("Step {i}"),
                command: (*command).into(),
                mode: *mode,
                submit: None,
                answers: None,
            })
            .collect(),
    }
}

fn outcomes(c: &Collector) -> Vec<StepOutcome> {
    c.snapshot()
        .events
        .into_iter()
        .filter_map(|e| match e {
            SessionEvent::StepFinished { outcome, .. } => Some(outcome),
            _ => None,
        })
        .collect()
}

fn wait_outcomes(c: &Collector, n: usize) -> Vec<StepOutcome> {
    assert!(
        c.wait_until(TIMEOUT, |s| {
            s.events.iter().filter(|e| matches!(e, SessionEvent::StepFinished { .. })).count() >= n
        }),
        "timed out waiting for {n} step(s); output:\n{}",
        c.plain_output()
    );
    outcomes(c)
}

fn wait_for_text(c: &Collector, needle: &str) {
    assert!(
        c.wait_until(TIMEOUT, |s| keyjutsu_core::headless::strip_ansi(&s.output).contains(needle)),
        "never saw {needle:?}; output:\n{}",
        c.plain_output()
    );
}

fn mash(session: &Session, keys: &str) {
    for c in keys.chars() {
        session.key(&KeyChord::char(c)).unwrap();
    }
}

fn state(session: &Session) -> ExecutionState {
    session.snapshot().expect("armed").state
}

#[test]
fn pwsh_is_a_normal_interactive_terminal() {
    let (session, out) = start(ShellKind::Pwsh);
    session.write_input(b"Write-Output ('hel' + 'lo')\r").unwrap();
    wait_for_text(&out, "\nhello");
    // Shell state persists between commands, as in any real session.
    session.write_input(b"$kjValue = 41 + 1\r").unwrap();
    session.write_input(b"Write-Output \"v=$kjValue\"\r").unwrap();
    wait_for_text(&out, "v=42");
    session.close();
}

#[test]
fn ansi_colour_reaches_the_renderer_untouched() {
    let (session, out) = start(ShellKind::Pwsh);
    session.write_input(b"Write-Host -ForegroundColor Red ('re' + 'd')\r").unwrap();
    wait_for_text(&out, "\nred");
    let raw = out.snapshot().output;
    // ConPTY re-encodes colours: on build 26200 red arrives as 256-colour
    // index 9 rather than the 16-colour code PowerShell asked for.
    let red = ["\x1b[91m", "\x1b[31m", "\x1b[38;5;9m", "\x1b[38;5;1m"];
    assert!(red.iter().any(|sgr| raw.contains(sgr)), "no red SGR in output: {}", raw.escape_debug());
    session.close();
}

#[test]
fn resizing_the_terminal_reaches_the_shell() {
    let (session, out) = start(ShellKind::Pwsh);
    session.resize(TerminalSize { rows: 40, cols: 97 }).unwrap();
    session.write_input(b"Write-Output \"w=$($Host.UI.RawUI.WindowSize.Width)\"\r").unwrap();
    wait_for_text(&out, "w=97");
    session.close();
}

/// Milestone 2's acceptance case, end to end against a real shell.
#[test]
fn mashed_keys_type_and_run_exactly_the_staged_command() {
    let (session, out) = start(ShellKind::Pwsh);
    session.arm(script(&[("Get-Service -Name Winmgmt", None)]), PerformanceConfig::default()).unwrap();
    let command_len = "Get-Service -Name Winmgmt".chars().count();
    // "asdfghjkl" repeated: none of these letters is in the output below.
    let mashing: String = "asdfghjkl".chars().cycle().take(command_len).collect();
    mash(&session, &mashing);
    assert_eq!(state(&session), ExecutionState::AwaitingExecution);
    mash(&session, "q");
    assert_eq!(wait_outcomes(&out, 1), vec![StepOutcome::Succeeded { exit_code: 0 }]);
    wait_for_text(&out, "Winmgmt");
    let plain = out.plain_output();
    assert!(!plain.contains("asdf"), "a physical key reached the shell:\n{plain}");
    assert_eq!(state(&session), ExecutionState::Complete);
    session.close();
}

#[test]
fn a_failing_command_fails_the_step_with_its_exit_code() {
    let (session, out) = start(ShellKind::Pwsh);
    session
        .arm(
            script(&[("cmd /c exit 3", Some(ExecutionMode::Direct)), ("Get-Date", None)]),
            PerformanceConfig::default(),
        )
        .unwrap();
    assert_eq!(wait_outcomes(&out, 1), vec![StepOutcome::Failed { exit_code: 3 }]);
    assert_eq!(state(&session), ExecutionState::Failed);
    // The keyboard is the operator's again and the second step never started.
    assert!(out.snapshot().events.contains(&SessionEvent::Released));
    assert!(!out.snapshot().events.contains(&SessionEvent::StepStarted { index: 1 }));
    session.write_input(b"Write-Output ('back' + 'again')\r").unwrap();
    wait_for_text(&out, "backagain");
    session.close();
}

#[test]
fn ctrl_c_interrupts_a_running_command() {
    let (session, out) = start(ShellKind::Pwsh);
    let started = std::time::Instant::now();
    session
        .arm(
            script(&[("Start-Sleep -Seconds 60", Some(ExecutionMode::Direct))]),
            PerformanceConfig::default(),
        )
        .unwrap();
    // Give the command time to begin; there is no pre-execution mark in V1.
    std::thread::sleep(Duration::from_millis(1500));
    session
        .key(&KeyChord { key: KeyName::Char('c'), ctrl: true, alt: false, shift: false, meta: false })
        .unwrap();
    let outcome = wait_outcomes(&out, 1);
    assert!(matches!(outcome[0], StepOutcome::Failed { .. }), "{outcome:?}");
    assert!(started.elapsed() < Duration::from_secs(30), "the sleep was not interrupted");
    session.close();
}

#[test]
fn raw_input_is_refused_while_a_performance_owns_the_keyboard() {
    let (session, out) = start(ShellKind::Pwsh);
    session.arm(script(&[("Get-Date", None)]), PerformanceConfig::default()).unwrap();
    assert!(matches!(session.write_input(b"Remove-Item x\r"), Err(CoreError::InputOwned)));
    // Renderer replies still get through, or the shell would stall.
    session.write_input(b"\x1b[I").unwrap();
    session.disarm();
    session.write_input(b"Write-Output ('free' + 'again')\r").unwrap();
    wait_for_text(&out, "freeagain");
    session.close();
}

#[test]
fn arming_is_refused_on_a_dirty_or_busy_line() {
    let (session, out) = start(ShellKind::Pwsh);
    session.write_input(b"Get-").unwrap();
    let err = session.arm(script(&[("Get-Date", None)]), PerformanceConfig::default()).unwrap_err();
    assert!(err.to_string().contains("not empty"), "{err}");
    // Clearing the line and submitting it empty makes it armable. Not with
    // Esc: ESC followed by CR reaches PSReadLine as Alt+Enter, which adds a
    // continuation line instead of submitting.
    session.write_input(b"\x7f\x7f\x7f\x7f").unwrap();
    session.write_input(b"\r").unwrap();
    assert!(session.wait_for_prompt(TIMEOUT), "no prompt came back:\n{}", out.plain_output());
    session.arm(script(&[("Get-Date", None)]), PerformanceConfig::default()).unwrap();
    session.close();
}

#[test]
fn hard_disarm_mid_typing_leaves_nothing_half_typed() {
    let (session, out) = start(ShellKind::Pwsh);
    session
        .arm(script(&[("Remove-Item -Path C:\\keyjutsu-never-exists", None)]), PerformanceConfig::default())
        .unwrap();
    mash(&session, "asdfgh");
    session
        .key(&KeyChord { key: KeyName::Char('K'), ctrl: true, alt: true, shift: true, meta: false })
        .unwrap();
    assert_eq!(state(&session), ExecutionState::Aborted);
    // A front end summarising the performance on release must already have
    // the final state: the snapshot comes first, then the release.
    let events = out.snapshot().events;
    let released = events.iter().position(|e| *e == SessionEvent::Released).expect("released");
    match &events[released - 1] {
        SessionEvent::Performance { snapshot } => assert_eq!(snapshot.state, ExecutionState::Aborted),
        other => panic!("expected the final snapshot before the release, got {other:?}"),
    }
    // If the partial "Remove" were still on the line this would run as
    // "RemoveWrite-Output ..." and fail.
    session.write_input(b"Write-Output ('clean' + 'line')\r").unwrap();
    wait_for_text(&out, "\ncleanline");
    session.close();
}

#[test]
fn auto_performance_runs_a_multi_step_script_by_itself() {
    let (session, out) = start(ShellKind::Pwsh);
    let config = PerformanceConfig {
        mode: ExecutionMode::AutoPerformance,
        cadence: keyjutsu_core::execution::Cadence {
            base_ms: 5,
            variance_ms: 2,
            punctuation_pause_ms: 5,
            boundary_pause_ms: 50,
        },
        ..PerformanceConfig::default()
    };
    session
        .arm(script(&[("Write-Output ('au' + 'to1')", None), ("Write-Output ('au' + 'to2')", None)]), config)
        .unwrap();
    assert_eq!(wait_outcomes(&out, 2), vec![StepOutcome::Succeeded { exit_code: 0 }; 2]);
    wait_for_text(&out, "auto1");
    wait_for_text(&out, "auto2");
    session.close();
}

#[test]
fn windows_powershell_5_1_reports_marks_and_exit_codes() {
    let (session, out) = start(ShellKind::WindowsPowershell);
    session
        .arm(
            script(&[
                ("$PSVersionTable.PSVersion.Major", Some(ExecutionMode::Direct)),
                ("cmd /c exit 4", Some(ExecutionMode::Direct)),
            ]),
            PerformanceConfig::default(),
        )
        .unwrap();
    assert_eq!(
        wait_outcomes(&out, 2),
        vec![StepOutcome::Succeeded { exit_code: 0 }, StepOutcome::Failed { exit_code: 4 }]
    );
    wait_for_text(&out, "\n5");
    session.close();
}

#[test]
fn cmd_runs_staged_commands_but_its_success_is_unverified() {
    let (session, out) = start(ShellKind::Cmd);
    session.arm(script(&[("echo kj-cmd^-ok", None)]), PerformanceConfig::default()).unwrap();
    mash(&session, &"z".repeat("echo kj-cmd^-ok".len() + 1));
    assert_eq!(wait_outcomes(&out, 1), vec![StepOutcome::Unverified]);
    wait_for_text(&out, "kj-cmd-ok");
    session.close();
}

#[test]
fn the_safe_demo_runs_in_direct_mode() {
    let (session, out) = start(ShellKind::Pwsh);
    let config = PerformanceConfig { mode: ExecutionMode::Direct, ..PerformanceConfig::default() };
    session.arm(keyjutsu_core::demo::safe_demo(ShellKind::Pwsh), config).unwrap();
    let results = wait_outcomes(&out, 3);
    assert!(results.iter().all(|o| matches!(o, StepOutcome::Succeeded { .. })), "{results:?}");
    wait_for_text(&out, "PSVersion");
    session.close();
}

#[test]
fn the_session_reports_the_shell_exiting() {
    let (session, out) = start(ShellKind::Pwsh);
    session.write_input(b"exit 7\r").unwrap();
    assert!(out.wait_until(TIMEOUT, |s| s.events.iter().any(|e| matches!(e, SessionEvent::Exited { .. }))));
    let code = out.snapshot().events.into_iter().find_map(|e| match e {
        SessionEvent::Exited { exit_code } => exit_code,
        _ => None,
    });
    assert_eq!(code, Some(7));
    assert!(session.arm(script(&[("Get-Date", None)]), PerformanceConfig::default()).is_err());
}
