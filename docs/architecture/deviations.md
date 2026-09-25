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

M8 has landed and this has not been done yet: `keyjutsu perform -c` still
performs the operator's text directly. It is the same authority as typing it,
so nothing is lost meanwhile, but the change is still owed.

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

## D12. Property tests stand in for fuzzing until Milestone 17

§55 asks for fuzzing of JSON and schema parsing and of the condition parser.
Milestone 3 has property tests instead: 8 properties, 2,000 generated cases
each, run on every `cargo test`. They check the same promises (hostile input
is refused cleanly, never with a panic; the same input always gets the same
answer) but they are not coverage-guided. cargo-fuzz needs nightly Rust and
its own harness; it is scheduled with the rest of the adversarial testing in
Milestone 17.

## D13. Critical steps are identified from the agent's risk label until Milestone 5

**Resolved in Milestone 5.** KeyJutsu now assesses risk itself and seals the
assessment with the plan; the agent's label can raise it but never lower it.
The original note follows.

§27 says risk must not rely solely on agent-provided labels. At Milestone 4 a
step needs its own typed confirmation when the agent proposed it as critical
or when KeyJutsu's recorded assessment says so, but KeyJutsu does not yet
assess risk itself. An agent that labels `Remove-Item -Recurse` as low risk is
not caught until the risk rules arrive with validation in Milestone 5.

## D14. Snapshot hashes are unkeyed until Milestones 10 and 14

A sealed snapshot's hashes detect accidental and naive edits. They cannot stop
someone with write access to the file, who can recompute them. Resisting that
needs a keyed MAC under a DPAPI-protected key (Milestone 14) checked by the
elevated broker (Milestone 10). Until then, a snapshot is only as trustworthy
as the folder it is stored in. See [ADR 0010](adr/0010-plan-hashing.md).

## D15. No file-copy staging

§13 lists "disposable copy testing": transform a copy of a configuration file,
parse the result, check required properties and that unrelated ones survive.
The plan schema has no structured way to express such a transform, so there is
nothing to stage. It needs a schema addition (a `transform` step kind with an
input file, an operation and a result check) before validation can do it.

## D16. Named facts are not collected

Conditions may name facts such as `docker.backend`. Validation has no fact
collectors yet, so every named fact is undecided and a step whose precondition
needs one goes to review. Collectors belong with agent integration (Milestone
6), where an investigating agent can propose them and KeyJutsu can run the
read-only ones itself.

## D17. The Cursor adapter is unverified

The Cursor agent CLI (`cursor-agent`) was not installed on the machine the
adapters were written on; only the Cursor editor was. Its invocation follows
Cursor's published usage but has not been checked against its `--help`.
`keyjutsu agents` reports it as unverified until it has been.

## D18. No adapter has been exercised against a live agent

Building Milestone 6 without the owner present, calling Codex, Claude Code,
Gemini or Copilot would have sent requests on their accounts without their
agreement. The adapters are tested against recorded answers in each CLI's
documented output shape. `keyjutsu agents check --live` is the live check, run
when the owner chooses.

## D19. One shell per plan

- **Requirement:** each step names its shell (§9), so a plan could mix pwsh,
  Windows PowerShell and cmd.
- **Limitation:** a performance is one terminal session with one shell. Running
  a cmd step inside pwsh would change what the step means; starting a second
  session part-way through would move the audience to a different terminal
  and lose the working directory and variables the earlier steps set up.
- **Evidence:** none needed yet: no example or agent-proposed plan so far
  mixes shells.
- **Options:** (a) refuse mixed plans at preflight; (b) one session per shell,
  switched between steps; (c) run the odd step through the main shell
  (`cmd /c …`), which changes the approved text.
- **Resolution taken:** (a). `preflight` refuses a plan that uses more than one
  shell and says so. (b) is the likely answer once the desktop app can show
  more than one terminal.

## D20. Checkpoints are plain JSON until Milestone 14

A checkpoint records which steps succeeded, and `--resume` skips those. It is
not hashed or signed, so someone who can write the file can mark a step as
done and have it skipped. It cannot make a changed step count, because each
result is matched to the step's hash in the approved snapshot, and it cannot
add a step. Protecting it belongs with the other local state in Milestone 14
(DPAPI-protected keys), alongside the snapshot MAC of D14.

A checkpoint that cannot be written does not stop the run; the run carries on,
and a later `--resume` will know less than it should. That should become a
warning on screen once there is somewhere to show it.

## D21. No Windows-native authentication integration yet

- **Requirement:** §25 prefers Windows-native authentication (SSPI/Kerberos,
  Windows Hello), then interactive browser or device flows, then a secure
  user-entered credential.
- **Limitation:** Milestone 9 builds the third: a secret or a user name and
  password typed into PowerShell's own masked prompt. Native mechanisms need
  a per-tool integration (which tool accepts a Kerberos ticket, which one
  opens Windows Hello), and none was needed by the plans built so far.
- **Evidence:** tools with their own browser or device sign-in (`az login`,
  `gh auth login`) already work as ordinary command steps, because the tool
  talks to the browser and KeyJutsu never sees the credential.
- **Options:** (a) a `windows_integrated` credential kind that asserts the
  current logon is used and checks it (for example with `klist`); (b) per-tool
  device-flow helpers; (c) leave both to command steps.
- **Resolution taken:** (c) for now. (a) is the natural next step when a plan
  needs it.
