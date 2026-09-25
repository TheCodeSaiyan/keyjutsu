# Milestone 10: the elevation broker

What was built, and what the "done when" in §60 rests on. The design is
[ADR 0011](adr/0011-elevation-broker.md).

## What it is

- **`keyjutsu-broker`** (new crate and binary): started elevated once per run,
  pinned to one snapshot, serving one authenticated client over a named pipe,
  running only approved Administrator steps checked against its own copy of
  their hashes.
- **Elevation preflight.** `keyjutsu run` and the desktop look for
  Administrator steps before anything starts; if there are any and KeyJutsu is
  not elevated, the broker is started then, with one UAC prompt, or nothing
  runs. Validation counts an Administrator step as READY only when the broker
  is installed (or KeyJutsu is elevated).
- **The executor** sends an Administrator step to the broker instead of the
  unelevated shell, shows what the broker's shell printed, and runs the
  step's checks as usual. Without a broker the step is refused before it
  runs. A broker failure after the step may have started leaves it in doubt.
- **`keyjutsu doctor`** and the desktop's first-run scan now report agents,
  the broker and encrypted storage as they are, rather than "not yet built".

## Done when

"The broker refuses altered commands, unknown plan hashes, unauthorized
operations, incompatible protocol versions. No arbitrary privileged string
execution API exists."

Over a real named pipe (`crates/keyjutsu-broker/tests/broker.rs`):

| Refused | Test |
| --- | --- |
| An altered command: the same step with a different command | `an_approved_administrator_step_runs_and_nothing_else_does` |
| An unknown plan hash | same |
| A step that does not need Administrator; a step that does not exist | same |
| A raw "execute this command" message, and a valid request with an extra `command` field | same |
| Anything before authentication | `another_protocol_version_is_refused_not_negotiated` |
| Protocol version 2 | same |
| The wrong secret; a process other than the launcher | `a_client_without_the_secret_or_from_another_process_is_refused` |
| A second pipe with the broker's name | `a_pipe_name_cannot_be_taken_over` |
| A snapshot other than the one launched for; an altered snapshot file | `the_broker_binary_refuses_a_snapshot_it_was_not_launched_for` |

The request types are closed (`deny_unknown_fields`, one kind of work), so
there is no message that carries a command. Each of the four central rules
(step hash, Administrator only, protocol version, launching process) was
removed in turn, and each removal made a test fail.

`an_administrator_step_goes_to_the_broker_not_the_unelevated_shell`: without a
broker the step is refused; with one, the broker is asked for exactly that
step of that snapshot by hash, and the command is never typed into the
unelevated shell.

## Real elevation, in Windows Sandbox

`tests/e2e/broker-trial/run.ps1`, 25 September 2026, networking off. Windows
Sandbox runs with UAC off, where everything is already elevated, so the
first run proved only the launch path. The trial now turns UAC on
(consent set to elevate without prompting), restarts the Sandbox and runs as
an ordinary user:

- the trial process ran at Medium integrity, not elevated;
- the broker, started through UAC, reported itself elevated;
- the unelevated client connected over the pipe;
- an altered version of the step was refused;
- the approved step wrote a key under `HKLM`, which only an Administrator
  can, and the key was there afterwards.

## Limits

- **Administrator steps are not performed** keystroke by keystroke; they run
  in the broker's shell and their output is shown
  ([deviation D28](deviations.md#d28-administrator-steps-run-in-the-brokers-shell)).
  The shown output includes the shell's echo of the command as the broker's
  shell drew it, which can repeat part of the command.
- **Artifacts are not handed to Administrator steps yet**; such a step is
  refused before it runs.
- **Recovery that needs elevation** (restoring a service or an HKLM value,
  Milestone 11) does not use the broker yet.
- **The pipe's access list** is enforced by Windows but not tested from
  another account.
- **A UAC prompt with consent required** was not exercised: the Sandbox trial
  set UAC to elevate without prompting so the test could run unattended.
