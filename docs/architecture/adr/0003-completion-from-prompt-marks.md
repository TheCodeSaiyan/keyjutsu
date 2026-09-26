# 0003: Command completion from nonce-stamped OSC 133 prompt marks

Status: accepted.

## Context

The engine must know when a command has finished, and whether it succeeded,
without guessing from timers and without running commands somewhere
other than the visible terminal. A shell running inside ConPTY gives no
direct signal; its process is still alive between commands.

## Decision

Launch each shell with a prompt wrapper that emits FinalTerm/OSC 133 marks,
the scheme Windows Terminal and VS Code use for shell integration:

- `D;<exit code>` when the previous command finished,
- `A` when the prompt starts, `B` when it ends and the input line begins.

PowerShell 7 and 5.1 get a wrapper around whatever `prompt` function the
user's profile installed, passed with `-EncodedCommand` so no layer of
command-line quoting can alter it. The wrapper reads `$?` first, then puts it
back with a suppressed `Write-Error` so oh-my-posh or Starship still see the
real success state. cmd.exe gets the marks in its `PROMPT` variable.

Every mark carries `kj=<nonce>`, 128 random bits generated per session. Marks
without the right nonce are passed through as ordinary output and never
trusted. KeyJutsu's own marks are removed from what the user sees.

## Evidence

The spike and the integration suite show, on Windows build 26200 through the
inbox ConPTY: pwsh 7.6.6 reports `D;0` after success and `D;1` after
`cmd /c exit 3`; Windows PowerShell 5.1 reports `D;4` after `cmd /c exit 4`;
cmd.exe reports marks but no code. These run on every `cargo test` in
`crates/keyjutsu-core/tests/real_shells.rs`.

## Consequences and limits

- **cmd.exe cannot report exit codes.** Its prompt is not re-evaluated for
  `%ERRORLEVEL%`. Steps in cmd finish as `Unverified`, never as success.
- **The exit code can be stale in PowerShell** after a cmdlet failure that
  follows an earlier native failure, because `$LASTEXITCODE` is not reset.
  The engine treats success versus failure (`$?`) as the signal and the number
  as evidence.
- **The nonce raises the bar; it does not close it.** A process running in the
  session can read its parent's command line and so the nonce. An approved
  command that set out to forge a completion mark could. The threat model
  records this; the mitigation is that such a command has been validated and
  approved before it runs.
- **A profile that replaces `prompt` after KeyJutsu's wrapper runs defeats
  the marks.** The shell never becomes ready, arming is refused, and the
  readiness probe reports it. The clean profile is the fallback.
- There is no pre-execution (`C`) mark, so KeyJutsu cannot tell when a
  submitted command actually started. The Ctrl+C test waits 1.5 seconds for
  that reason; a PSReadLine hook could provide it later.
