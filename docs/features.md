# What you get

The [README](../README.md) is the short version. This is each part of
KeyJutsu, why it works the way it does, and the commands.

## A real terminal, performed

KeyJutsu runs your own shells, PowerShell 7, Windows PowerShell 5.1 or `cmd`,
in Windows' pseudo-console, the same one Windows Terminal uses. Colour, resize,
Ctrl+C and full-screen programs behave as they do anywhere else, because
there's no imitation shell in between.

While a performance is armed, each key you press types the next character
of an approved command. The key decides when, never what: it's been tested
with random keys against every state the engine can be in, and only the staged
text ever reaches the shell. A step moves on when the shell says the command
finished, through a mark its prompt prints with a secret per session, not
after a timer.

```powershell
keyjutsu demo --clean
```

[Your first performance](guides/first-performance.md) ·
[Performing](performing.md)

## Plans from the agents you already use

KeyJutsu doesn't bring its own AI. It runs the agent CLI you already have,
Codex, Claude Code, Gemini, GitHub Copilot or Cursor, under your account, and
keeps each one to investigating: every one is started in its own read-only
mode, and what it returns is a plan: steps and their commands, which
KeyJutsu reads rather than runs. An agent's
claim that a step is ready, proved or approved is refused as part of the
format, because those are KeyJutsu's to decide.

Before anything is sent, you see what would go. Secret-looking
things (tokens, keys, private keys, `password=` lines) are redacted from
everything sent: the task, pasted text, files, your guidance and the plan
itself.

At the last live check, Codex, Claude Code and GitHub Copilot passed.
Gemini's plan mode can be switched off silently in its own settings, so
KeyJutsu refuses its answer whenever it falls back out of read-only mode.

```powershell
keyjutsu agents
keyjutsu plan propose "Find out why the Print Spooler keeps stopping" --agent claude --out spooler.json
```

[Getting a plan from an agent](guides/planning.md) ·
[Agents](agent-integrations/README.md)

## Validation that runs nothing

Every command in a plan is read, not run: its syntax through PowerShell's own
parser, whether each command and parameter exists on this machine, the tools
and versions it needs, the privilege it needs, and its risk, which KeyJutsu
rates itself. An agent can raise a step's risk but not lower it.

The one thing that executes is `-WhatIf`, for built-in management commands
with plain arguments, and it's switched on with `$WhatIfPreference` rather
than by adding `-WhatIf` to the command. Added to the end, a `#` earlier in
the command would turn it into part of a comment, and the "dry run" would be
a real one; there's a test that tries exactly that.

```powershell
keyjutsu plan validate spooler.json
```

## Approval that's sealed to what it approved

Approving seals the plan into a snapshot: each step bound to a hash of what
it runs and everything before it. Change a step and its approval, and every
later step's, falls away. A critical step, KeyJutsu's own judgement rather
than the agent's, needs its own typed phrase, both when it's approved and
again before it runs if the approval is over an hour old.

The approval is also recorded in an encrypted store that only your Windows
account on this machine can unlock, and `run` refuses a snapshot without that
record. Before that, a snapshot edited and re-sealed with KeyJutsu's own
library passed every check.

```powershell
keyjutsu plan approve spooler.json --out spooler.approved.json
```

## Execution that checks itself

Each step runs in its approved folder, then its checks run: paths, services,
file hashes, JSON values, ports, HTTP. A failed check stops the plan, and the
keyboard comes back to you. Before each step, a checkpoint records that it
started, so after a crash nothing is assumed: a step that was running is "in
doubt" until you say how it went.

Before anything runs, the machine is compared with the one the plan was
approved on. A changed tool version sends the steps that depend on it back
to validation instead of letting them run on assumptions.

```powershell
keyjutsu run spooler.approved.json
```

[Running a plan](guides/running-a-plan.md)

## Credentials you type yourself

A step that needs a password or token hands the keyboard back: you press
Enter and type it into PowerShell's own masked prompt. KeyJutsu never types
it and never stores it. Keys pressed after your answer are held,
so a stray one can't spill into the next prompt.

## Administrator steps, asked for once

Steps that need Administrator run through KeyJutsu's elevation broker,
started with one UAC prompt before the performance, so no prompt appears
halfway through. The broker runs only approved steps, identified by the
snapshot's hash and the step's; there is no request that carries a command.
On a clean Windows 11 with UAC on, it ran an approved step as Administrator
and refused an altered one.

## Recovery, when you say so

A step can declare what it will change and how to undo it. Just before it
runs, those files, registry values or service states are captured, with their
hashes. After a failure, nothing is undone until you've seen the plan for
undoing it and confirmed.

```powershell
keyjutsu recover spooler.approved.json
keyjutsu recover spooler.approved.json --confirm
```

[When a run stops](guides/when-a-run-stops.md)

## Git that tells its changes from yours

In a repository, KeyJutsu records every changed file first. Afterwards, only
files that differ from that record count as its changes, and its diff is taken
against it, so your own uncommitted work is never reverted or blamed on the
run. `--isolate worktree` keeps the run off your checkout entirely.

```powershell
keyjutsu run fix.approved.json --isolate worktree
keyjutsu git diff fix.approved.json
```

## Downloads pinned before approval

A plan that downloads something names it with a SHA-256. `plan stage` fetches
and checks it before approval, and it's checked again just before the step,
so a URL that starts serving something else after you approved is caught.
Nothing is downloaded while a plan runs.

## Plans that cross a restart

A plan can be written in phases, with a Windows restart, a sign-out, a new
shell, or a WSL or Docker restart between them. KeyJutsu stops at the
boundary, records what should change, and on resume checks the restart
happened, the machine still matches, and everything the earlier phases did
still holds. It has been run across a real Windows restart.

## History and Techniques

Every run is recorded, encrypted, from the desktop app or the CLI; a CLI run
can opt out with `--ephemeral`. A run that
worked can become a Technique, with parameters, to use again. A Technique
never runs because it worked before: it makes a draft that's validated and
approved here, with a note of which steps are on unfamiliar ground.

[Reusing a plan that worked](guides/techniques.md)

## Diagnostics you read before sharing

When you need help, a diagnostic bundle gives someone the versions, the
readiness checks and how each shell behaved, without your tasks, commands,
output, name or secrets. You see all of it before it's saved, and KeyJutsu
sends it nowhere: in the app, **Diagnostics** in the Terminal screen's side
panel has **Preview bundle** and then **Save bundle**, and the CLI has
`keyjutsu diagnostics preview` and `keyjutsu diagnostics save FILE`.

[Commands](commands.md#keyjutsu-diagnostics-preview)

## A desktop app and a CLI, with one set of rules

The desktop app (New task, Plan and Terminal) and the `keyjutsu` CLI drive
the same Rust core, so there's one implementation of what may reach a shell.
The desktop stops at a restart boundary without resuming past it; the CLI
can resume there.

## Checked against its own rules

The eight release gates are named lists of tests, 80 in all,
run by `pnpm release:gates` and in CI; a renamed or deleted test fails its
gate. Untrusted inputs are fuzzed as part of the ordinary suite: plans,
snapshots, broker requests and terminal output. Fuzzing the terminal scanner
found that any command printing an unfinished escape sequence could make a
step hang until its timeout; it was fixed with a test that fails on the old
code. What isn't covered is listed in the
[threat model](../THREAT_MODEL.md#not-covered).
