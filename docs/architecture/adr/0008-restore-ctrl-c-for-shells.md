# 0008: Clear the inherited Ctrl+C-ignore flag before starting shells

Status: accepted.

## Context

The first run of the real-shell suite failed one test: `0x03` written to
ConPTY did not interrupt `Start-Sleep -Seconds 60`. Writing the key as a
win32-input-mode event did not help either.

The cause was not ConPTY. Windows passes a process's "ignore Ctrl+C"
attribute to every child it creates, and a process started in a new process
group gets that attribute. The test runner had been launched that way, so
every shell it started ignored Ctrl+C. The spike confirmed it: with
`SetConsoleCtrlHandler(NULL, FALSE)` called first, the same `0x03` stopped
`Start-Sleep` at once; without it, nothing happened for the full sleep.

## Decision

`keyjutsu_terminal::platform::restore_ctrl_c_for_children()` makes that call
once, before the first shell is spawned. It is the workspace's only `unsafe`
block, and the lint that denies `unsafe` is lifted for that one function
alone, with a safety comment.

## Consequences

This matters outside tests too. CI agents, IDE task runners and some
launchers start programs in a new process group; without this, Ctrl+C in a
KeyJutsu terminal started from one of them would silently not work, and
Ctrl+C has to be a genuine interrupt.

KeyJutsu's own process now handles Ctrl+C normally. The desktop app has no
console, and the CLI reads keys in raw mode where Ctrl+C is a key rather than
a signal, so neither is affected in practice.
