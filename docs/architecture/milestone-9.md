# Milestone 9: credential gates

What was built, and what the "done when" in §60 rests on. The design is
[ADR 0015](adr/0015-credentials-through-the-shells-masked-prompt.md).

## What it is

- **A credential step says what it asks for, never the value.** The plan
  schema gains `credential` (`variable`, `prompt`, `kind`, optional
  `username` and `target_id`), required on a credential step and refused on
  any other.
- **The operator answers in PowerShell's own masked prompt.** KeyJutsu writes
  `$VAR = Read-Host -AsSecureString -Prompt '…'` (or `Get-Credential`) directly,
  with no fake typing, and the operator's keys go to that prompt. The shell
  shows `*`, keeps the value as a `SecureString` or `PSCredential`, and
  PSReadLine does not record it. KeyJutsu never holds the secret.
- **Staged typing stops and waits.** The step starts only when the operator
  presses Enter; keys still being mashed are swallowed, and neither a front
  end nor the executor can start it for them. The CLI's console title says
  "Credential required: … Stop typing, then press Enter."; the desktop
  overlay says the same.
- **Nothing typed after the answer reaches the shell.** Once the operator has
  pressed Enter as many times as the prompt needs (one, or two for a user name
  and a password), keys are swallowed until the command finishes.
- **The credential is forgotten.** When the plan completes or fails, KeyJutsu
  runs `Remove-Variable` for every credential it asked for. A resumed run asks
  again.
- **cmd is refused**, by validation (INVALID) and again by preflight: `set /p`
  would show the secret as it is typed.

## Done when

"A secret can be entered and used without appearing in plan JSON, agent
context, terminal replay, logs, telemetry."

| Where | Status | Evidence |
| --- | --- | --- |
| Entered | Met | `a_credential_is_entered_and_used_without_being_seen_or_kept` (real pwsh, Performance mode, the operator mashing before and after); the CLI end to end in `run_asks_for_a_credential_in_the_shells_masked_prompt`; `a_user_name_and_password_are_asked_for_in_turn`. |
| Used | Met | The next step prints the secret's length (`length 12`) through `NetworkCredential`, proving the value arrived intact without printing it. |
| Plan JSON | Met | The schema has no field for a value; both tests check the plan and the sealed snapshot. |
| Agent context | Met by construction | The secret exists only inside the shell. Agents see plans and pasted context, neither of which can contain it; there is no path from the terminal to an agent. |
| Terminal replay | Met | The raw terminal output (everything ConPTY sent, before any stripping) is checked in both tests: only `*` appears. KeyJutsu keeps no replay of its own yet; when it does (M14), it records output, which is masked. |
| Logs | Met | KeyJutsu writes no logs; the checkpoint and every execution event are checked for the secret. PowerShell's history is checked too (`Get-History`). |
| Telemetry | Met trivially | There is no telemetry (invariant 8). |
| Afterwards | Met | The variable is gone once the plan ends (`Test-Path variable:KJ_TOKEN` is `False`); `a_resumed_run_asks_for_the_credential_again`. |

## Checked by breaking it

Each of these made a test fail when the guard was removed:

- **Starting the prompt for the operator.** `Session::arm` sends `Start` to
  every user-input step, so the first version opened the credential prompt at
  once; keys still being mashed would have gone into it. The engine now
  ignores `Start` for a line that asks the operator.
- **Keys after the answer.** Found by the real-shell test: in the time between
  the operator's final Enter and the shell reporting back, mashed keys were
  forwarded and landed on the next prompt line (`qqqqqq'length ' + …`), which
  broke the next step. The answer count fixes it.
- **Carrying a credential over on resume**: the resumed run never asked, and
  the next step would have found no variable.
- **The cmd check in preflight** (validation now catches it first as well).

## Limits

- **Windows-native authentication is not integrated.** §25 prefers
  SSPI/Kerberos, Windows Hello and browser or device flows. A tool that does
  its own browser or device sign-in (`az login`, `gh auth login`) works as an
  ordinary command step, because KeyJutsu never sees what it asks for; there
  is nothing KeyJutsu-specific for these yet
  ([deviation D21](deviations.md#d21-no-windows-native-authentication-integration-yet)).
- **Nothing is stored between runs**, so nothing is written to Windows
  Credential Manager or protected with DPAPI. Every run asks again.
- **A plan can print the secret by using the variable carelessly.**
  Validation does not yet flag a step that turns `$VAR` back into plain text
  and writes it to the screen.
- **Disarming leaves the variable in the shell.** After a disarm the operator
  owns the terminal and KeyJutsu types nothing into it; the variable lasts
  until that shell exits (the CLI's shell exits when the operator leaves it).
- **Windows PowerShell 5.1's credential dialog is not tested.** Its
  `Get-Credential` opens a Windows dialog rather than prompting in the
  console. The step still finishes when the dialog closes, but no test covers it.
- **The desktop overlay's credential notice has not been seen on screen.**
- **One intermittent test failure, cause not proven.** In the full suite,
  `a_resumed_run_asks_for_the_credential_again` failed in 2 of 4 runs, taking
  about four minutes each time; it never failed on its own. The test's
  operator waited for the prompt's text, which the echoed command line also
  contains, so it could start typing before `Read-Host` was reading. It now
  waits for `Read-Host`'s own `…: `, and the suite has been clean since (8
  runs, plus the full workspace run). That fits the symptom but does not prove
  it was the cause; the failure output of those two runs was not kept.
