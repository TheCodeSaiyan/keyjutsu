# Milestone 8: execution

What was built, and what each "done when" in §60 rests on. Milestone 8 came
before Milestone 7 on purpose ([dependency graph](dependency-graph.md)): it
needs only the terminal, approval and validation, and it can be proven against
real shells without anyone looking at a screen.

## What it is

`keyjutsu-core::execute`, and `keyjutsu run` in the CLI.

- **Preflight.** A snapshot runs only if it was validated before it was
  sealed, every step is READY, and every step uses the same shell. The CLI
  also re-collects the environment fingerprint and refuses to start if
  anything an approved step depends on has changed since approval (§33).
- **One step at a time.** Each plan step becomes a staged performance: its
  command lines, then its visible validation unless the plan hides it. The
  performance engine from Milestone 2 does the typing, so every mode works
  exactly as it does for the demo.
- **Runtime validation.** When the shell reports a step's lines finished, its
  internal checks run against the machine itself: exit code, service state,
  path, file SHA-256, a JSON value, a TCP port, an HTTP status. With
  `timeout_seconds`, failing checks are retried once a second until the
  timeout, which is how a step waits for a service to come up.
- **Branches from real outcomes.** What runs next is decided by the plan's
  walk (Milestone 3) from what actually happened, never by the agent (§10).
- **Failure pause.** A failed line or a failed check stops the plan. The
  outcome says what was expected and what happened; nothing further runs.
- **Checkpoints.** Written before and after every step
  (`<snapshot>.checkpoint.json`, replaced atomically). A step that was running
  when KeyJutsu stopped is recorded as in doubt.
- **Repair and re-arm.** `keyjutsu plan revise`, validate and approve give a
  new snapshot; `keyjutsu run <new> --resume <old checkpoint>` reuses the old
  run's results, but only for steps that succeeded and whose step hash is
  unchanged. The new run writes its own checkpoint next to the new snapshot. Anything the
  revision touched runs again.
- **In-doubt steps are settled by the operator.** A resumed run will not
  start while a step is in doubt. The operator checks the machine and passes
  `--settle STEP=succeeded` or `--settle STEP=failed`; the first counts it as
  done, the second runs it again.
- **Per-step mode override.** `--mode` (or the plan's default mode) sets the
  mode for the run; a step's own `execution_mode` wins over it.
- **Nothing is printed into the terminal during the run.** The shell's own
  output is the performance. Progress goes to the console title, and the
  outcome is printed after the shell exits.

## Done when

| §60 criterion | Status | Evidence |
| --- | --- | --- |
| A multi-step approved task executes in Performance, Assisted, Auto Performance and Direct from the same approved plan | Met | `the_same_snapshot_runs_in_every_mode`: one three-step plan with internal checks and visible validation, validated, approved and sealed as the CLI does it, then run in all four modes against real pwsh; the file it writes is checked each time, and in the two key-driven modes the test mashes keys and checks none reached the shell. |
| Execution checkpoints | Met | `checkpoints_are_written_as_the_plan_runs`; the CLI end-to-end test checks the file exists after `keyjutsu run`. |
| Runtime validation | Met, partly exercised | `a_failing_internal_check_fails_the_step`, `runtime_checks_look_at_the_machine_itself` (exit code, SHA-256, JSON value, TCP open and closed), `a_check_is_waited_for_up_to_its_timeout`. Service and HTTP checks are not exercised by a test. |
| Failure pause | Met | `a_failure_halts_the_plan_and_reports_expected_versus_actual`. |
| Repair and re-arm | Met | `a_repaired_plan_resumes_without_rerunning_unchanged_steps`, `a_changed_step_runs_again_even_though_it_succeeded_before`, `a_step_left_in_doubt_blocks_until_the_operator_settles_it`. |
| Per-step mode override | Met | `a_step_can_override_the_run_mode`: a Direct step inside a Performance run finishes with nobody pressing keys. |

The CLI is covered end to end by `run_executes_an_approved_snapshot_in_performance_mode`:
`keyjutsu plan approve`, then `keyjutsu run` inside a nested pseudo-console,
with keys mashed from the moment it starts.

## Checked by breaking it

- **A shell that exited mid-step counted as a success.** Found while checking
  the per-step override test by breaking it: with the override removed, the
  run should have waited for keys forever, but it reported the step succeeded
  after about 90 seconds without having typed anything. The shell had gone
  away; the engine moves to `FAILED` when it does, without a result for any
  line, and the executor read "no failed line, no failed check" as success. A
  step now succeeds only when the performance completed with a result for
  every line; otherwise the run is aborted, and the step is left in doubt if a
  command had been submitted. `a_shell_that_exits_mid_step_is_never_a_success`
  fails without the fix, recording the step as succeeded.
- **Keys pressed before the first step is armed were typed into the shell.**
  The CLI e2e test found it: the operator's first mashed keys arrived before
  the controller had armed anything, reached pwsh as real input, and the dirty
  line then stopped the performance arming, so nothing ran. While a plan is
  running, the CLI now holds keys that arrive with nothing armed. The test
  fails without the hold.
- **Per-step override:** with the step's mode ignored, the override test fails.

## Limits

- **One shell per plan** ([deviation D19](deviations.md#d19-one-shell-per-plan)).
- **Checkpoints are plain JSON** until Milestone 14
  ([D20](deviations.md#d20-checkpoints-are-plain-json-until-milestone-14)).
- **The `WAITING` state is not entered.** Waiting for a check happens after
  the step's performance has completed, in the executor, so the engine is
  already `COMPLETE`; the wait is reported as an execution event instead. The
  desktop overlay will need to show it from that event (Milestone 7).
- **cmd steps are unverified, and an unverified step with no internal checks
  is not a failure.** cmd reports no exit codes (D3), so such a step is
  recorded as succeeded with no exit code. Add internal checks to cmd steps.
- **No gate for critical steps at execution time yet.** Approval requires the
  typed confirmation for every critical step and the snapshot re-checks it on
  load, but `keyjutsu run` does not ask again just before a critical step
  runs. That belongs with the plan workspace (Milestone 7), where there is
  somewhere to ask.
- **No desktop UI for this yet** (Milestone 7).
- **Nothing runs elevated.** A step that needs elevation fails as it would if
  the operator typed it; the broker is Milestone 10.
- **Recovery is not attempted.** A failed plan stops; rollback is Milestone 11.
