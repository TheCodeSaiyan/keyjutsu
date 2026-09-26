//! Behaviour of the Performance Mode engine, driven through its public API
//! exactly as keyjutsu-core drives it.

#![allow(clippy::unwrap_used)] // Helpers outside #[test] functions may unwrap too.

use keyjutsu_execution::*;
use keyjutsu_terminal::{KeyChord, KeyName, ShellMark};

fn script(commands: &[&str]) -> StagedScript {
    StagedScript {
        steps: commands
            .iter()
            .enumerate()
            .map(|(i, c)| StagedStep {
                id: format!("s{i}"),
                title: format!("Step {i}"),
                command: (*c).to_owned(),
                mode: None,
                submit: None,
                answers: None,
            })
            .collect(),
    }
}

fn engine(commands: &[&str], config: PerformanceConfig) -> PerformanceEngine {
    let mut e = PerformanceEngine::new(script(commands), config).unwrap();
    e.handle(Input::Arm);
    e
}

fn press(e: &mut PerformanceEngine, chord: KeyChord) -> Vec<Action> {
    e.handle(Input::Key(KeyInput::from_chord(&chord, &Bindings::default())))
}

fn mash(e: &mut PerformanceEngine, keys: &str) -> Vec<u8> {
    keys.chars().flat_map(|c| written(&press(e, KeyChord::char(c)))).collect()
}

fn written(actions: &[Action]) -> Vec<u8> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::Write(b) => Some(b.clone()),
            _ => None,
        })
        .flatten()
        .collect()
}

fn finished(code: Option<i32>) -> Input {
    Input::Shell(ShellMark::CommandFinished { exit_code: code })
}

fn disarm_chord() -> KeyChord {
    KeyChord { key: KeyName::Char('K'), ctrl: true, alt: true, shift: true, meta: false }
}

fn assert_all_transitions_legal(e: &PerformanceEngine) {
    for (from, to) in e.history() {
        assert!(from.can_transition_to(*to), "{from:?} -> {to:?}");
    }
}

/// The case the engine exists for: staged `Get-Service`, the user types
/// `asdfghjkl`, and the shell receives exactly the staged characters.
#[test]
fn mashing_arbitrary_keys_delivers_exactly_the_staged_command() {
    let mut e = engine(&["Get-Service"], PerformanceConfig::default());
    let sent = mash(&mut e, "asdfghjkl");
    assert_eq!(sent, b"Get-Servi");
    assert_eq!(e.state(), ExecutionState::Typing);

    let sent = mash(&mut e, "zx");
    assert_eq!(sent, b"ce");
    assert_eq!(e.state(), ExecutionState::AwaitingExecution);

    // Only now can a key deliver the staged Enter.
    let sent = mash(&mut e, "q");
    assert_eq!(sent, b"\r");
    assert_eq!(e.state(), ExecutionState::Executing);
    assert_all_transitions_legal(&e);
}

#[test]
fn enter_pressed_mid_command_advances_instead_of_submitting() {
    let mut e = engine(&["Get-Date"], PerformanceConfig::default());
    mash(&mut e, "abc");
    for _ in 0..5 {
        let out = press(&mut e, KeyChord::plain(KeyName::Enter));
        assert!(!written(&out).contains(&b'\r'), "Enter leaked through as a submit");
    }
    assert_eq!(e.state(), ExecutionState::AwaitingExecution);
    assert_eq!(written(&press(&mut e, KeyChord::plain(KeyName::Enter))), b"\r");
}

#[test]
fn no_sequence_of_keys_can_submit_an_incomplete_command() {
    // Every ordinary key, repeated, up to one short of the full command.
    let command = "Get-ChildItem -Force";
    let keys = ["a", "Z", " ", "\n", ";", "1"];
    for key in keys {
        let mut e = engine(&[command], PerformanceConfig::default());
        let mut sent = Vec::new();
        for _ in 0..command.chars().count() {
            let chord = if key == "\n" {
                KeyChord::plain(KeyName::Enter)
            } else {
                KeyChord::char(key.chars().next().unwrap())
            };
            sent.extend(written(&press(&mut e, chord)));
        }
        assert_eq!(sent, command.as_bytes(), "key {key:?}");
        assert_eq!(e.state(), ExecutionState::AwaitingExecution);
    }
}

#[test]
fn require_enter_ignores_other_keys_at_the_boundary() {
    let config = PerformanceConfig { submit: SubmitPolicy::RequireEnter, ..PerformanceConfig::default() };
    let mut e = engine(&["ls"], config);
    mash(&mut e, "xx");
    assert!(mash(&mut e, "yyyy").is_empty());
    assert_eq!(e.state(), ExecutionState::AwaitingExecution);
    assert_eq!(written(&press(&mut e, KeyChord::plain(KeyName::Enter))), b"\r");
}

#[test]
fn auto_submit_sends_enter_after_the_last_character() {
    let config = PerformanceConfig { submit: SubmitPolicy::AutoSubmit, ..PerformanceConfig::default() };
    let mut e = engine(&["ls"], config);
    assert_eq!(mash(&mut e, "xx"), b"ls\r");
    assert_eq!(e.state(), ExecutionState::Executing);
}

#[test]
fn turbo_advances_a_word_and_assisted_a_small_burst() {
    let turbo = PerformanceConfig { advance: AdvanceStyle::Turbo, ..PerformanceConfig::default() };
    let mut e = engine(&["Get-Service -Name docker"], turbo);
    assert_eq!(mash(&mut e, "a"), b"Get-Service ");
    assert_eq!(mash(&mut e, "a"), b"-Name ");
    assert_eq!(mash(&mut e, "a"), b"docker");
    assert_eq!(e.state(), ExecutionState::AwaitingExecution);

    let assisted = PerformanceConfig { mode: ExecutionMode::Assisted, ..PerformanceConfig::default() };
    let mut e = engine(&["Get-Date -Format o"], assisted);
    assert_eq!(mash(&mut e, "a"), b"Get");
    assert_eq!(mash(&mut e, "a"), b"-Da");
    assert_eq!(mash(&mut e, "a"), b"te ");
    let rest = mash(&mut e, "aaaaaaaa");
    assert_eq!(rest, b"-Format o\r");
}

#[test]
fn the_hard_disarm_chord_works_in_every_state_and_erases_partial_input() {
    // Typing: the partial line is erased so nothing half-typed is left behind.
    let mut e = engine(&["Get-Service"], PerformanceConfig::default());
    mash(&mut e, "abcd");
    let out = press(&mut e, disarm_chord());
    assert_eq!(written(&out), vec![0x7f; 4]);
    assert_eq!(e.state(), ExecutionState::Aborted);
    assert!(!e.owns_input());
    assert!(out.contains(&Action::Released));

    // Executing: the running command is left alone, not interrupted.
    let mut e = engine(&["ls"], PerformanceConfig::default());
    mash(&mut e, "abc");
    assert_eq!(e.state(), ExecutionState::Executing);
    let out = press(&mut e, disarm_chord());
    assert!(written(&out).is_empty());
    assert_eq!(e.state(), ExecutionState::Aborted);

    // Complete, holding input: disarm hands the terminal back.
    let mut e = engine(&["ls"], PerformanceConfig::default());
    mash(&mut e, "abc");
    e.handle(finished(Some(0)));
    assert_eq!(e.state(), ExecutionState::Complete);
    assert!(e.owns_input());
    press(&mut e, disarm_chord());
    assert!(!e.owns_input());

    // Paused, armed and even before arming.
    for setup in [Input::Pause, Input::Start] {
        let mut e = engine(&["ls"], PerformanceConfig::default());
        e.handle(setup);
        press(&mut e, disarm_chord());
        assert_eq!(e.state(), ExecutionState::Aborted);
    }
    let mut e = PerformanceEngine::new(script(&["ls"]), PerformanceConfig::default()).unwrap();
    press(&mut e, disarm_chord());
    assert_eq!(e.state(), ExecutionState::Aborted);
}

/// Found driving the desktop app: the overlay pauses the engine, and
/// disarming from the overlay left the half-typed command on the prompt.
#[test]
fn disarming_while_paused_mid_command_erases_the_partial_input() {
    let mut e = engine(&["Get-ComputerInfo"], PerformanceConfig::default());
    mash(&mut e, "abcdefghij");
    e.handle(Input::Pause);
    assert_eq!(e.state(), ExecutionState::Paused);
    let out = press(&mut e, disarm_chord());
    assert_eq!(written(&out), vec![0x7f; 10]);

    // Revalidation from a pause clears the line too.
    let mut e = engine(&["Get-ComputerInfo"], PerformanceConfig::default());
    mash(&mut e, "abc");
    e.handle(Input::Pause);
    assert_eq!(written(&e.handle(Input::RequireRevalidation)), vec![0x7f; 3]);

    // After Ctrl+C the shell has already dropped the line: nothing to erase.
    let ctrl_c = KeyChord { key: KeyName::Char('c'), ctrl: true, alt: false, shift: false, meta: false };
    let mut e = engine(&["Get-ComputerInfo"], PerformanceConfig::default());
    mash(&mut e, "abc");
    press(&mut e, ctrl_c);
    assert!(written(&press(&mut e, disarm_chord())).is_empty());
}

#[test]
fn escape_is_swallowed_while_typing_and_forwarded_to_a_running_command() {
    let mut e = engine(&["less file"], PerformanceConfig::default());
    mash(&mut e, "ab");
    assert!(written(&press(&mut e, KeyChord::plain(KeyName::Escape))).is_empty());
    assert_eq!(e.state(), ExecutionState::Typing, "Esc must not disarm");
    mash(&mut e, "abcdefghij");
    assert_eq!(e.state(), ExecutionState::Executing);
    assert_eq!(written(&press(&mut e, KeyChord::plain(KeyName::Escape))), b"\x1b");
}

#[test]
fn keys_mashed_while_a_command_runs_do_not_reach_it() {
    let mut e = engine(&["Start-Sleep 5"], PerformanceConfig::default());
    mash(&mut e, &"x".repeat(14));
    assert_eq!(e.state(), ExecutionState::Executing);
    assert!(mash(&mut e, "qwerty").is_empty());
}

#[test]
fn ctrl_c_is_a_real_interrupt() {
    let ctrl_c = KeyChord { key: KeyName::Char('c'), ctrl: true, alt: false, shift: false, meta: false };

    // While executing it reaches the process; the shell then reports failure.
    let mut e = engine(&["Start-Sleep 30", "Get-Date"], PerformanceConfig::default());
    mash(&mut e, &"x".repeat(15));
    assert_eq!(written(&press(&mut e, ctrl_c)), b"\x03");
    let out = e.handle(finished(Some(1)));
    assert!(out.contains(&Action::StepFinished { index: 0, outcome: StepOutcome::Failed { exit_code: 1 } }));
    assert_eq!(e.state(), ExecutionState::Failed);

    // While typing, the shell drops the line, so the step is retyped whole.
    let mut e = engine(&["Get-Date"], PerformanceConfig::default());
    mash(&mut e, "abc");
    assert_eq!(written(&press(&mut e, ctrl_c)), b"\x03");
    assert_eq!(e.state(), ExecutionState::Paused);
    assert!(mash(&mut e, "zzz").is_empty(), "paused engines type nothing");
    e.handle(Input::Resume);
    assert_eq!(mash(&mut e, "abcdefgh"), b"Get-Date");
    assert_all_transitions_legal(&e);
}

#[test]
fn steps_advance_only_when_the_shell_reports_completion() {
    let mut e = engine(&["one", "two"], PerformanceConfig::default());
    mash(&mut e, "aaaa");
    assert_eq!(e.state(), ExecutionState::Executing);
    // Marks that are not completion do not advance anything.
    e.handle(Input::Shell(ShellMark::PromptStart));
    e.handle(Input::Shell(ShellMark::CommandStart));
    assert_eq!(e.state(), ExecutionState::Executing);
    assert!(mash(&mut e, "bbb").is_empty());

    let out = e.handle(finished(Some(0)));
    assert!(
        out.contains(&Action::StepFinished { index: 0, outcome: StepOutcome::Succeeded { exit_code: 0 } })
    );
    assert!(out.contains(&Action::StepStarted { index: 1 }));
    assert_eq!(e.state(), ExecutionState::Typing);
    assert_eq!(mash(&mut e, "cccc"), b"two\r");
    e.handle(finished(Some(0)));
    assert_eq!(e.state(), ExecutionState::Complete);
    assert_all_transitions_legal(&e);
}

#[test]
fn a_failed_step_stops_the_performance_and_returns_control() {
    let mut e = engine(&["bad", "never"], PerformanceConfig::default());
    mash(&mut e, "aaaa");
    let out = e.handle(finished(Some(2)));
    assert_eq!(e.state(), ExecutionState::Failed);
    assert!(out.contains(&Action::Released));
    assert!(!out.contains(&Action::StepStarted { index: 1 }));
    assert!(mash(&mut e, "zzzz").is_empty());
}

#[test]
fn a_shell_without_exit_codes_is_recorded_as_unverified_not_passed() {
    let mut e = engine(&["dir"], PerformanceConfig::default());
    mash(&mut e, "aaaa");
    e.handle(finished(None));
    assert_eq!(e.snapshot().outcomes, vec![StepOutcome::Unverified]);
}

#[test]
fn auto_performance_types_by_ticks_and_ignores_keys() {
    let config = PerformanceConfig { mode: ExecutionMode::AutoPerformance, ..PerformanceConfig::default() };
    let mut e = PerformanceEngine::new(script(&["ls", "pwd"]), config).unwrap();
    let out = e.handle(Input::Arm);
    assert!(out.iter().any(|a| matches!(a, Action::ScheduleTick(_))));
    assert!(mash(&mut e, "zzz").is_empty(), "auto mode is watch-only");

    let mut sent = Vec::new();
    // Ticks drive typing and submission; nothing advances past Executing.
    for _ in 0..20 {
        sent.extend(written(&e.handle(Input::Tick)));
    }
    assert_eq!(sent, b"ls\r");
    assert_eq!(e.state(), ExecutionState::Executing);

    e.handle(finished(Some(0)));
    let mut sent = Vec::new();
    for _ in 0..20 {
        sent.extend(written(&e.handle(Input::Tick)));
    }
    assert_eq!(sent, b"pwd\r");
    e.handle(finished(Some(0)));
    assert_eq!(e.state(), ExecutionState::Complete);
    assert_all_transitions_legal(&e);
}

#[test]
fn per_step_modes_override_the_global_mode() {
    let mut s = script(&["auto", "direct", "", "typed"]);
    s.steps[0].mode = Some(ExecutionMode::AutoPerformance);
    s.steps[1].mode = Some(ExecutionMode::Direct);
    s.steps[2].mode = Some(ExecutionMode::UserInput);
    let mut e = PerformanceEngine::new(s, PerformanceConfig::default()).unwrap();
    e.handle(Input::Arm);

    let mut sent = Vec::new();
    for _ in 0..10 {
        sent.extend(written(&e.handle(Input::Tick)));
    }
    assert_eq!(sent, b"auto\r");

    // The Direct step is sent whole the moment the previous one passes.
    let out = e.handle(finished(Some(0)));
    assert_eq!(written(&out), b"direct\r");
    assert_eq!(e.state(), ExecutionState::Executing);

    // The user-input step forwards real keys, including Enter.
    e.handle(finished(Some(0)));
    assert_eq!(e.state(), ExecutionState::AwaitingUserInput);
    assert_eq!(mash(&mut e, "hi"), b"hi");
    assert_eq!(written(&press(&mut e, KeyChord::plain(KeyName::Enter))), b"\r");

    e.handle(finished(Some(0)));
    assert_eq!(e.state(), ExecutionState::Typing);
    assert_eq!(mash(&mut e, "zzzzzz"), b"typed\r");
    assert_all_transitions_legal(&e);
}

#[test]
fn a_pause_requested_mid_command_takes_effect_before_the_next_step() {
    let mut e = engine(&["one", "two"], PerformanceConfig::default());
    mash(&mut e, "aaaa");
    e.handle(Input::Pause);
    assert_eq!(e.state(), ExecutionState::Executing);
    e.handle(finished(Some(0)));
    assert_eq!(e.state(), ExecutionState::Paused);
    assert!(mash(&mut e, "zz").is_empty());
    e.handle(Input::Resume);
    assert_eq!(e.state(), ExecutionState::Typing);
    assert_eq!(mash(&mut e, "zzzz"), b"two\r");
    assert_all_transitions_legal(&e);
}

#[test]
fn the_shell_exiting_fails_the_performance() {
    let mut e = engine(&["exit"], PerformanceConfig::default());
    mash(&mut e, "aaaaa");
    e.handle(Input::ShellExited);
    assert_eq!(e.state(), ExecutionState::Failed);
    assert!(!e.owns_input());
}

#[test]
fn revalidation_erases_partial_input_and_releases_the_keyboard() {
    let mut e = engine(&["Get-Date"], PerformanceConfig::default());
    mash(&mut e, "ab");
    let out = e.handle(Input::RequireRevalidation);
    assert_eq!(written(&out), vec![0x7f; 2]);
    assert_eq!(e.state(), ExecutionState::RevalidationRequired);
    assert!(!e.owns_input());
}

#[test]
fn unicode_commands_are_delivered_one_character_per_key() {
    let mut e = engine(&["echo café ✓"], PerformanceConfig::default());
    let sent = mash(&mut e, &"x".repeat(11));
    assert_eq!(String::from_utf8(sent).unwrap(), "echo café ✓");
}

#[test]
fn the_overlay_chord_is_reported_and_types_nothing() {
    let mut e = engine(&["ls"], PerformanceConfig::default());
    mash(&mut e, "a");
    let overlay = KeyChord { key: KeyName::Char('K'), ctrl: true, alt: false, shift: true, meta: false };
    let out = press(&mut e, overlay);
    assert_eq!(out, vec![Action::OverlayRequested]);
}

#[test]
fn the_snapshot_names_the_next_step_for_the_overlay() {
    let mut e = engine(&["first", "second"], PerformanceConfig::default());
    mash(&mut e, "a");
    assert_eq!(e.snapshot().next_step_title.as_deref(), Some("Step 1"));
    mash(&mut e, "aaaaa");
    e.handle(finished(Some(0)));
    assert_eq!(e.snapshot().step_index, 1);
    assert_eq!(e.snapshot().next_step_title, None, "the last step has nothing after it");
}

/// A credential step: KeyJutsu's command asks, the operator answers.
fn asking(command: &str) -> PerformanceEngine {
    let mut s = script(&[command]);
    s.steps[0].mode = Some(ExecutionMode::UserInput);
    let mut e = PerformanceEngine::new(s, PerformanceConfig::default()).unwrap();
    e.handle(Input::Arm);
    e
}

#[test]
fn a_line_that_asks_the_operator_waits_for_enter_and_is_never_performed() {
    let mut e = asking("$t = Read-Host -AsSecureString -Prompt 'Token'");
    assert!(e.asks_operator());

    // Keys still being mashed for the previous step start nothing.
    assert!(mash(&mut e, "asdfjkl").is_empty());
    assert_eq!(e.state(), ExecutionState::Armed);

    // Enter starts it: the command is written whole, not typed out.
    let out = press(&mut e, KeyChord::plain(KeyName::Enter));
    assert_eq!(written(&out), b"$t = Read-Host -AsSecureString -Prompt 'Token'\r");
    assert_eq!(e.state(), ExecutionState::AwaitingUserInput);

    // From then on the keys are the operator's answer, delivered as typed.
    assert_eq!(mash(&mut e, "s3cret"), b"s3cret");
    assert_eq!(written(&press(&mut e, KeyChord::plain(KeyName::Enter))), b"\r");
    e.handle(finished(Some(0)));
    assert_eq!(e.state(), ExecutionState::Complete);
    assert_all_transitions_legal(&e);
}

#[test]
fn a_line_that_asks_the_operator_is_not_started_for_them() {
    let mut e = asking("$t = Read-Host -AsSecureString");
    assert!(written(&e.handle(Input::Start)).is_empty(), "a front end cannot start it for the operator");
    for _ in 0..5 {
        assert!(written(&e.handle(Input::Tick)).is_empty());
    }
    assert_eq!(e.state(), ExecutionState::Armed);
}

#[test]
fn cancelling_the_prompt_fails_the_step() {
    let mut e = asking("$t = Read-Host -AsSecureString");
    press(&mut e, KeyChord::plain(KeyName::Enter));
    let ctrl_c = KeyChord { key: KeyName::Char('c'), ctrl: true, alt: false, shift: false, meta: false };
    assert!(!written(&press(&mut e, ctrl_c)).is_empty(), "Ctrl+C reaches the prompt");
    e.handle(finished(Some(1)));
    assert_eq!(e.state(), ExecutionState::Failed);
}

#[test]
fn keys_after_the_last_answer_do_not_reach_the_next_prompt() {
    let mut s = script(&["$c = Get-Credential"]);
    s.steps[0].mode = Some(ExecutionMode::UserInput);
    s.steps[0].answers = Some(2);
    let mut e = PerformanceEngine::new(s, PerformanceConfig::default()).unwrap();
    e.handle(Input::Arm);
    e.handle(Input::Key(KeyInput::from_chord(&KeyChord::plain(KeyName::Enter), &Bindings::default())));

    // User name, Enter, password, Enter: all of it is the operator's.
    assert_eq!(mash(&mut e, "bob"), b"bob");
    press(&mut e, KeyChord::plain(KeyName::Enter));
    assert_eq!(e.state(), ExecutionState::AwaitingUserInput);
    assert_eq!(mash(&mut e, "pw"), b"pw");
    press(&mut e, KeyChord::plain(KeyName::Enter));

    // Answered: anything else is swallowed until the shell reports back.
    assert_eq!(e.state(), ExecutionState::Executing);
    assert!(mash(&mut e, "qqqq").is_empty());
    e.handle(finished(Some(0)));
    assert_eq!(e.state(), ExecutionState::Complete);
    assert_all_transitions_legal(&e);
}
