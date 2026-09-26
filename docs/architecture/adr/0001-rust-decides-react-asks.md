# 0001: The Rust core is the authority; the React front end only asks

Status: accepted.

## Context

The stack is Tauri, Rust, React, TypeScript, xterm.js and ConPTY, and React
must not be authoritative for approval, execution state, hashes, security
policy, risk, credentials, broker authorisation or command execution. A webview is also the part of the app most exposed to
content KeyJutsu does not control: terminal output is rendered there, and
later so will agent text.

## Decision

- Every decision about what reaches a shell is made in `keyjutsu-core` or
  below. The Tauri command layer is a set of wrappers that forward requests
  and report refusals.
- The window never sends a script for the built-in demo; it names the source
  (`ScriptSource::SafeDemo`) and the core builds the script.
- Raw input from the window is refused while a performance owns the keyboard,
  apart from the replies a terminal renderer sends by itself (cursor
  position, focus, device attributes), which are recognised by pattern in
  Rust (`is_terminal_report`).
- The window gets an opaque session number, never a handle it could misuse.
- No Tauri plugin that reaches the file system or starts processes is
  enabled. The capability file grants `core:default` and permission to set
  the window title, nothing else.
- UI state such as "armed" is a display of snapshots the core sends. The Arm
  button's enabled state is advice; the core rechecks everything on arm.

## Consequences

A compromised renderer can still type into an *unarmed* terminal, exactly as
the user could. That is inherent in a terminal app and is recorded in the
threat model. What it cannot do is arm a performance on a dirty or busy line,
push text into a line the engine owns, or arm anything that is not an
approved snapshot.

The cost is an IPC round trip per keypress while armed. Measured by eye in the
desktop app it is not perceptible; it has not been measured properly and
should be, with a typing-latency test.
