# 0011: A separate, narrowly scoped elevation broker

Status: accepted. First recorded as a proposal before any code, so nothing
built before it would make it harder.

## Decision

- **KeyJutsu always runs unelevated.** A step marked `privilege:
  administrator` goes to `keyjutsu-broker.exe`, a separate binary installed
  next to KeyJutsu, started elevated through Windows' own "run as
  administrator" once per run, before the session starts: one UAC prompt,
  never in the middle of a performance.
- **It is pinned to one snapshot.** The launch names the snapshot file and
  its hash. The broker verifies the whole snapshot (every hash) and refuses
  to start unless its hash is the one it was launched for, so a file swapped
  between approval and elevation is refused (exit codes say which).
- **Its pipe** admits only the launching Windows account and SYSTEM (a
  protected DACL built from the account's SID), rejects remote clients, and
  is created with `FILE_FLAG_FIRST_PIPE_INSTANCE`, so a name someone else
  created first makes the broker exit instead of sharing it. The name is 128
  random bits.
- **The client is checked three ways:** the connecting process must be the
  one that launched the broker (`GetNamedPipeClientProcessId`), it must know
  the per-launch 256-bit secret, and it must speak protocol version 1. A
  different version is refused, not negotiated.
- **There is one kind of work:** *run approved step X of snapshot S, whose
  hash is H*. The broker checks S is its snapshot, X exists and is an
  Administrator step, and H is X's hash in its own copy. An altered command
  changes the hash and is refused. Requests are closed JSON types with no
  unknown fields; there is no request that carries a command string.
- **It runs the step in its own elevated shell**, directly (not as a typing
  performance), and returns each line's result and what the shell printed,
  which KeyJutsu shows in the terminal. KeyJutsu runs the step's checks and
  records the result as for any step.
- **It is short-lived:** it serves one connection and exits when that ends,
  or after a minute if nobody connects.

## The open question, answered

The visible terminal and the real execution should correspond. An
Administrator step cannot be typed into the operator's unelevated shell and
run elevated, so it runs in the broker's shell and its output is shown in the
terminal, marked as the broker's. It is not performed keystroke by keystroke.

## Consequences

- The secret travels on the broker's command line. A process at Medium
  integrity cannot read the command line of a High-integrity process, so
  another unelevated program of the same user cannot read it; an elevated one
  could, but it is already an Administrator.
- Same-user malware can always ask UAC for elevation itself; the broker adds
  no way round that boundary and removes none. What it guarantees is that
  what runs elevated is exactly what was approved.
- The launch pins the snapshot hash the operator's KeyJutsu checked, which
  is what UAC is then asked about. KeyJutsu checks it against the approval
  recorded in its encrypted store before launching.
- An Administrator step that uses a download gets a copy the broker made
  and checked, in a folder only Administrators and SYSTEM can write to, never
  the staged file in the operator's profile, which anything running as the
  operator can change. The launcher says where the store is; that path is not
  trusted, because only a copy matching the snapshot's pinned hash is handed
  over.
- The pipe's DACL and the launching-process check are enforced by Windows;
  the tests cover the process check and the name squatting, not an attempt
  from another account.
