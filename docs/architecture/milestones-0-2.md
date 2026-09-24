# Milestones 0 to 2: plan and record

The first implementation plan (§64 item 7), and what each "done when" rests
on. A criterion marked met has a test or an observation behind it, named
here; one that is not met says what is missing.

## Why this order

The specification asks for a working real-terminal slice before any AI
orchestration, and it is the right call for a reason beyond tidiness: if the
illusion cannot be made to work on real shells, nothing built above it
matters. So the order was: prove ConPTY and completion detection in a spike,
build the terminal crate, build the engine as a pure state machine, join them
in the core, then put both front ends on top and drive each of them for real.

## Milestone 0: repository foundation

| Done when | Status | Evidence |
| --- | --- | --- |
| Repository builds cleanly on Windows | Met | `cargo clippy --workspace --all-targets -- -D warnings` clean; `pnpm -r typecheck`, `lint` and `build` clean. |
| Frontend launches | Met | Desktop app launched and driven; screenshots of first run, workspace and armed takeover taken during the session. |
| CLI launches | Met | `keyjutsu doctor` runs the full readiness scan in about 1.4 seconds. |
| Shared Rust core consumed by both | Met | Both depend only on `keyjutsu-core`; neither contains session or engine logic. |
| CI validates Rust and TypeScript | Written, not yet run | `.github/workflows/ci.yml`. The repository has no remote, so it has not run on GitHub yet. |
| Architecture docs describe dependency boundaries | Met | [overview.md](overview.md), [dependency-graph.md](dependency-graph.md). |

## Milestone 1: real terminal

| Done when | Status | Evidence |
| --- | --- | --- |
| Usable as a normal interactive terminal | Met | `pwsh_is_a_normal_interactive_terminal`; typed into the desktop app with `SendKeys`, output and PSReadLine colouring shown. |
| PTY output is real | Met | Every assertion in `real_shells.rs` reads what the shell printed; the probe uses commands whose output differs from their own text, so seeing the output proves the command ran. |
| Shell state persists naturally | Met | Same test: a variable set in one command is read by the next. |
| Ctrl+C works | Met | `ctrl_c_interrupts_a_running_command`, after [ADR 0008](adr/0008-restore-ctrl-c-for-shells.md). |
| Resizing works | Met | `resizing_the_terminal_reaches_the_shell` (pwsh reports the new width); the desktop terminal reflowed when the sidebar came back. |
| ANSI output works | Met | `ansi_colour_reaches_the_renderer_untouched`. |
| Normal keyboard interaction works | Met | Desktop driven by real keystrokes; CLI end-to-end test sends keys through the console input stack. |
| PowerShell 5.1 and cmd | Met | `windows_powershell_5_1_reports_marks_and_exit_codes`, `cmd_runs_staged_commands_but_its_success_is_unverified`. |
| Terminal profile detection | Partial | Windows Terminal's default profile is read and applied. See [D6](deviations.md#d6-terminal-profile-matching-is-partial). |

## Milestone 2: performance engine

| Done when | Status | Evidence |
| --- | --- | --- |
| `asdfghjkl` types exactly `Get-Service` | Met | `mashing_arbitrary_keys_delivers_exactly_the_staged_command` (engine), `mashed_keys_type_and_run_exactly_the_staged_command` (real pwsh), and by hand in the desktop app: 45 keys of `asdfghjkl` produced the first 45 characters of the demo command. |
| No incomplete command can execute | Met | `no_sequence_of_keys_can_submit_an_incomplete_command`, `enter_pressed_mid_command_advances_instead_of_submitting`; mutation-checked. |
| Hard disarm always works | Met | Engine, real-shell, CLI (as a Win32 key event) and desktop (as a real keypress). |
| Pure, Assisted, Auto and Direct modes | Met | `turbo_advances_a_word_and_assisted_a_small_burst`, `auto_performance_runs_a_multi_step_script_by_itself`, `the_safe_demo_runs_in_direct_mode`. |
| Operator overlay | Met in the desktop | Opened with Ctrl+Shift+K while armed; pauses, shows progress, resumes or disarms. The CLI toggles pause instead ([ADR 0006](adr/0006-disarm-chord-and-escape.md)). |
| Command-boundary and Enter handling | Met | `require_enter_ignores_other_keys_at_the_boundary`, `auto_submit_sends_enter_after_the_last_character`. |
| Execution state machine | Met | [execution-state-machine.md](execution-state-machine.md); transitions checked on every change and in tests. |
| Per-step mode overrides | Met | `per_step_modes_override_the_global_mode`. |

## Bugs found by running it rather than reading it

Each was reproduced first, fixed, and left behind a test that fails without
the fix:

1. Ctrl+C did not interrupt anything, because the inherited Ctrl+C-ignore
   flag reached the shell ([ADR 0008](adr/0008-restore-ctrl-c-for-shells.md)).
2. The readiness probe reported cmd as broken: ConPTY draws cmd's output with
   cursor moves, so line-by-line matching never found it.
3. A trailing comma followed by a comment in Windows Terminal's settings
   defeated the JSON-with-comments reader.
4. Disarming from the overlay left the half-typed command on the prompt.
5. The desktop's "disarmed after…" notice never appeared, because the release
   event reached the window before the final state did.

## What comes next

Milestone 3, the plan model. Its schema is already drafted
([docs/schemas/plan.md](../schemas/plan.md)); the Rust side is parsing,
graph construction with cycle detection, and a condition evaluator for the
closed vocabulary the schema defines.
