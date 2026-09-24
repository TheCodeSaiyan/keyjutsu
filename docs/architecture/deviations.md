# Deviations from the specification

Where the build differs from the specification, it is written down here rather
than quietly reinterpreted. Platform conflicts use the specification's own
format (requirement, limitation, evidence, options, resolution).

## D1. `Get-PSVersionTable` is not a cmdlet

The safe demo (§42) suggests `Get-PSVersionTable`, which does not exist. The
demo reads the `$PSVersionTable` variable instead:
`$PSVersionTable | Select-Object PSVersion, PSEdition`.

## D2. Operator-authored staged commands

Milestone 2 needs something to perform before plans (M3), approval (M4) and
execution of approved plans (M8) exist. Two sources are allowed: the built-in
read-only demo, and commands the operator types themselves (`keyjutsu perform
-c …` and "My own commands" in the desktop app).

These carry no approval and are labelled as such in both front ends. They
grant no authority the operator lacks: they run exactly as if the operator had
typed them into the same terminal. When M8 lands, the operator-authored
source should become "make a one-step draft plan", going through validation
and approval like any other.

## D3. cmd.exe cannot report exit codes

- **Requirement:** each step's success is validated (§23), and a green result
  has meaning (§2.4).
- **Platform limitation:** cmd's `PROMPT` is expanded without variable
  substitution, so `%ERRORLEVEL%` cannot appear in the completion mark.
- **Evidence:** in the spike, cmd drew `D` marks with no parameter after
  `cmd /c exit 3`; `cmd_runs_staged_commands_but_its_success_is_unverified`
  checks it on every run.
- **Options:** (a) append `& echo` of the error level to each command, which
  changes the approved text and what the audience sees; (b) wrap each command
  in a child `cmd /c`, which changes its semantics (`cd` and `set` stop
  persisting); (c) record cmd steps as unverified.
- **Resolution taken:** (c). Steps finish as `Unverified`, never as success.
  Runtime validation (M8) can add an internal check after the command where
  one exists. PowerShell 7 remains the preferred shell (§14).

## D4. No `packages/ui` yet

Its only consumer would be the desktop app, where those components live.
Created when a second consumer exists ([ADR 0005](adr/0005-crates-arrive-with-consumers.md)).

## D5. The opening prompt ("What do you want KeyJutsu to do?") is not shown

§8's initial screen asks for a task to plan. Planning needs agents (M6), so a
task box now would be a control that does nothing. The app opens on the
first-run readiness scan (§42) and then the workspace; the task box arrives
with M6/M7.

## D6. Terminal profile matching is partial

Windows Terminal's default profile (font, size, colour scheme, cursor shape,
padding) is read and applied. Not yet done: conhost defaults for users without
Windows Terminal; built-in schemes other than Campbell and Campbell PowerShell
(others fall back to Campbell rather than being guessed at); PSReadLine,
oh-my-posh and Starship are detected but not yet reproduced in a cloned
compatibility profile. The choices offered today are the user's profile and
the clean profile; §16's third option, a compatible cloned profile, is
outstanding.

## D7. CI does not yet sign, bundle or produce an SBOM

§56 lists signing, installers, SBOMs and published checksums. They belong to
the release pipeline (M16). CI today formats, lints, type-checks, tests,
checks generated types and schemas, audits dependencies and builds the
desktop binary with `--no-bundle`.

## D8. TypeScript 6.0 rather than 7

typescript-eslint supports TypeScript below 6.1. See
[ADR 0007](adr/0007-typescript-generated-from-rust.md).

## D9. The `WAITING` state is defined but not entered

It belongs to runtime validation (M8), which waits for conditions such as a
service becoming healthy. The state and its transitions are in the table now
so the machine does not change shape later.

## D10. Humanised typos are not implemented

§19 lists "optional safe typo/correction effects". Not built: every typo
effect writes and then erases characters that are not in the approved
command, which needs care to stay provably harmless. Cadence variance and
punctuation and boundary pauses are implemented.

## D11. Licence is GPL-3.0-only, not Apache-2.0

§1 names Apache-2.0. At the owner's request the licence is GPL-3.0-only, so
modified versions cannot be distributed closed while KeyJutsu stays open
source and keeps every dependency it uses. Reasoning and the dependency check
are in [ADR 0012](adr/0012-gpl-3-licence.md).
