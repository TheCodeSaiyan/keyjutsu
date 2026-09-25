# Milestone 11: recovery

What was built, and what the "done when" in §60 rests on. Milestone 10 (the
elevated broker) is skipped for now: it needs an elevated process and UAC,
which cannot be exercised or checked without the owner at the machine.

## What it is

`keyjutsu-core::recovery`, the checkpoint's `captures`, and
`keyjutsu recover` in the CLI.

- **Capture before the step runs.** A step whose `recovery` lists captures
  has them taken just before it is armed:
  - **file**: the bytes are copied into the run's recovery folder
    (`<checkpoint>.recovery/`), the copy is read back and its SHA-256 compared
    with the original's; a file that did not exist is recorded as absent;
  - **registry value** (`HKCU:\Key\Value` or `HKLM:\Key\Value`): whether it
    existed, its type and its value;
  - **service state**: status and start type;
  - **package version**: recorded, but cannot be restored yet.

  If any capture cannot be made and verified, the step does not run and the
  plan stops as blocked: a recovery that was never prepared is not a recovery.
- **Recovery is checked by validation too.** A restore with nothing captured,
  a commands recovery with no commands, a malformed registry target or a
  folder given as a file capture makes the step INVALID; a package-version
  capture puts it in review.
- **Nothing is rolled back unless the operator says so.** After a failure,
  `keyjutsu run` lists the four choices from §29: diagnose first, review the
  recovery plan (`keyjutsu recover <snapshot>`), roll back (the same with
  `--confirm`), or stop without rolling back.
- **The recovery plan** covers every step that ran or was running when the
  run stopped, latest first: restore what was captured, run the step's
  approved recovery commands, or say why the step cannot be recovered.
  `--step` limits it to chosen steps. A step whose hash has changed since it
  ran is refused: recover with the snapshot it ran under.
- **Recovery is verified.** A restored file must hash to what was captured,
  and a restored registry value must read back identical. Then the step's own
  `recovery.validation` checks run. Recovery stops at the first step that
  does not recover, as execution does.
- **Recovery commands are execution.** They come from the approved snapshot
  (they are part of the step's hash, and validation parses them), and they
  run visibly in a terminal, in Direct mode.
- **A failure hands the keyboard back.** Found while building this; see below.

## Done when

"A failed reversible task can demonstrate successful operator-controlled
recovery without touching unrelated pre-existing state."

`a_failed_reversible_task_is_recovered_without_touching_anything_else` (real
pwsh, real file system, real registry under a test key of its own in HKCU):

- a five-step plan changes a file, changes a registry value, creates a file,
  creates a registry value, then fails its last step's check;
- the test confirms each change really happened, and that nothing was undone
  by the failure itself;
- the recovery plan lists the steps latest first, and says the failing step
  declared no recovery;
- after recovery the file is byte-for-byte as it was (CRLF included), the
  created file is gone, the registry value is back to `String:old`, the
  created value is gone;
- the file next to it has the same bytes and the same modification time, and
  the registry value next to the changed one is still `DWord:7`.

Operator control is covered by
`only_the_steps_the_operator_chooses_are_recovered` and, end to end through
the CLI, `a_failed_run_is_recovered_only_when_the_operator_confirms`: the run
fails, `keyjutsu recover` shows the plan and changes nothing, and only
`--confirm` restores the file.

## Checked by breaking it

- Using a backup without checking its hash: `a_backup_changed_since_it_was_taken_is_not_used` fails.
- Not removing a registry value that did not exist before: the done-when test fails.
- Running a step whose capture failed: `a_step_whose_recovery_cannot_be_prepared_does_not_run` fails.
- **A failed check kept the keyboard.** The CLI test found it: a step that
  fails only its internal checks has a performance that completed, and a
  completed performance keeps the keyboard until disarmed. After the failure
  the operator's keys went nowhere, with nothing on screen to say why. Any
  outcome other than completion now disarms; `a_failing_internal_check_fails_the_step`
  checks the keyboard is free, and fails without the change.

## Limits

- **No desktop rollback UI.** §60 asks for an operator rollback UI; the CLI's
  review-then-confirm is it for now. The desktop workspace is Milestone 7.
- **Services mostly need elevation to restore.** Capture works as a standard
  user; `Set-Service` on most services does not. Until the broker (Milestone
  10), restoring a service usually fails and is reported as failed.
- **HKLM values need elevation to restore**, for the same reason.
- **Package versions are not restored**, only recorded, and validation puts
  such a step in review.
- **A service that did not exist before is not deleted**, and a registry key
  created by a step is left in place (only its value is removed). Both are
  deliberate: removing more than was captured could touch state the plan
  never declared.
- **Backups are plain copies** in the recovery folder, as the checkpoint is
  plain JSON (D20). A backup of a file holding secrets is as exposed as the
  file was. Protecting both belongs with Milestone 14.
- **Only what a step declares is captured.** A step that changes something it
  did not declare cannot have that change undone; `expected_effects` are not
  yet compared with the captures.
