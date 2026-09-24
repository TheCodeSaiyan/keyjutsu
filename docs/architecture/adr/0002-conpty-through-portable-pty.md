# 0002: ConPTY through `portable-pty`

Status: accepted, Milestone 1.

## Context

Performance Mode has to run real commands in a real terminal (§15). On
Windows that means ConPTY: `CreatePseudoConsole`, a pair of pipes and a
process started with the pseudo-console attribute. That is a few hundred lines
of `unsafe` Win32 code to write and keep correct, including the awkward parts
(closing the console so the output stream ends, resize, sideloaded
`conpty.dll`).

## Options

1. Hand-written bindings with `windows-sys`.
2. `portable-pty` 0.9, the pseudo-terminal layer from WezTerm, with ConPTY,
   Unix PTY and sideloaded-ConPTY backends.

## Decision

Use `portable-pty`, wrapped by `keyjutsu-terminal::PtySession` so nothing else
depends on it directly.

It is mature, it keeps the workspace free of `unsafe` apart from one
documented call ([ADR 0008](0008-restore-ctrl-c-for-shells.md)), and its Unix
backend is the obvious route for the later Linux and macOS targets.

## What the spike showed

Before committing, a spike ran pwsh 7.6.6 and cmd.exe through
`portable-pty` on Windows build 26200. What it established:

- ConPTY opens with a cursor-position query (`ESC [ 6 n`) and **blocks until
  it is answered**. xterm.js answers it; headless sessions and the CLI must,
  so `MarkScanner` can intercept it.
- ConPTY asks for win32-input-mode (`ESC [ ? 9001 h`). xterm.js ignores the
  request, and plain VT input still works.
- OSC 133 prompt marks pass through intact, which [ADR 0003](0003-completion-from-prompt-marks.md)
  depends on.
- ConPTY re-encodes colours: `Write-Host -ForegroundColor Red` arrives as
  256-colour index 9 (`ESC[38;5;9m`), not as the 16-colour code asked for.
- It positions cmd's output with cursor movements rather than line breaks, so
  output must be searched rather than split into lines.

## Consequences

`portable-pty` creates the pseudo-console with the inherit-cursor,
resize-quirk and win32-input-mode flags, which KeyJutsu does not control.
If one of those ever conflicts with what KeyJutsu needs, the wrapper is where
a hand-written backend would slot in.
