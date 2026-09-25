# Commands

Every `keyjutsu` command and option, grouped by what you'd use it for. Each
command's `--help` says the same, and `pnpm docs:check` fails if this page
names a command or option the CLI doesn't have.

Options that take a plan or snapshot file take its path. Exit codes: 0 when
it did what was asked, 1 when it refused or something it checked failed,
2 when the command line itself was wrong.

## Checking the machine

### `keyjutsu doctor`

Checks this machine is ready: Windows, the architecture, ConPTY, each shell's
staged typing and exit codes, agents, the elevation broker and encrypted
storage. Each check is done for real. Exits 1 if any check fails.

| Option | |
| --- | --- |
| `--json` | the full report as JSON |

### `keyjutsu agents`

Lists every supported agent: installed or not, its version, whether it looks
signed in (by whether its credential file exists; the file is never opened),
and how it's kept read-only. An agent whose version differs from the one its
adapter was checked against is flagged.

| Option | |
| --- | --- |
| `--json` | the list as JSON |

### `keyjutsu agents check --live`

Sends each installed agent a tiny request, on your accounts, and checks the
adapter still works with its version. `--live` is required, so it can't
happen by accident.

### `keyjutsu diagnostics preview`

Prints a diagnostic bundle for someone helping you with a problem: the
KeyJutsu and Windows versions, every `doctor` check, how each shell behaved
in the pseudo-console, the agents and their versions, and how many runs and
Techniques the history holds. It leaves out tasks, plans, commands,
terminal contents, what runs printed, environment variables and
credentials. Your profile folder, your Windows user name and the computer's
name are replaced with placeholders, and anything shaped like a secret is
redacted. Nothing is sent anywhere.

### `keyjutsu diagnostics save FILE`

Writes the same bundle to `FILE`, as plain text, so you can read it before
you send it. The machine is checked again as it's saved, so timings can
differ from an earlier preview; nothing else is added.

| Option | |
| --- | --- |
| `--force` | replace an existing file |

## Performing

### `keyjutsu demo`

The safe demo: three read-only commands, performed.

### `keyjutsu perform -c COMMAND`

Performs commands you supply. Repeat `-c` for several, run in order. They're
yours: no plan, no validation and no approval, and they run as if you'd typed
them.

### `keyjutsu shell`

An ordinary interactive shell through KeyJutsu's terminal, with nothing
performed.

Options shared by `demo`, `perform` and `shell`:

| Option | |
| --- | --- |
| `--shell pwsh\|powershell\|cmd` | which shell; PowerShell 7 if installed, otherwise Windows PowerShell |
| `--clean` | skip your profile, history predictions and history saving |
| `--mode performance\|assisted\|auto\|direct` | how commands are delivered (`demo` and `perform`); see [Performing](performing.md) |
| `--turbo` | a word per key rather than a character (`demo` and `perform`) |
| `--submit any-key\|enter\|auto` | what submits a finished command (`demo` and `perform`) |

## Plans

### `keyjutsu plan propose TASK --agent AGENT --out FILE`

Asks an agent to investigate, read-only, and propose a plan. Without `--send`
it shows exactly what would be sent and stops.

| Option | |
| --- | --- |
| `--agent` | `codex`, `claude`, `gemini`, `copilot` or `cursor` |
| `--send` | actually send it |
| `--file FILE` | a file to include, redacted; repeatable |
| `--folder DIR` | a folder the agent may investigate; it's started there |
| `--out FILE` | where to write the plan |

### `keyjutsu plan revise FILE --step STEP --guidance TEXT --agent AGENT --out FILE`

Asks an agent to redo one step, with your guidance and what validation found.
No other step may change. `--session ID` names a recorded run in which that
step failed: the agent is also shown what it printed, redacted, to diagnose
from. `--task` gives the task if the plan's title doesn't say it well enough;
`--send` as above.

### `keyjutsu plan review FILE --agent AGENT`

Asks a second agent to challenge the plan. It can't change it. `--record FILE`
writes the plan with the review recorded in its provenance; `--task` and
`--send` as above.

### `keyjutsu plan check FILE`

Checks a plan against the schema and its structure, and shows the order its
steps would run in. `--stored` checks it as a stored plan, which may carry
KeyJutsu's own state; by default it's checked as an agent's proposal, which
may not.

### `keyjutsu plan validate FILE`

Validates a plan against this machine: syntax, commands, parameters, tools,
preconditions, privilege, risk and, where it's trustworthy, a `-WhatIf` dry
run. Nothing the plan names is run.

| Option | |
| --- | --- |
| `--no-dry-run` | skip `-WhatIf` dry runs |
| `--json` | the full report as JSON |

### `keyjutsu plan approve FILE --out SNAPSHOT`

Validates again, then approves every step and seals the plan into a snapshot,
recording the approval in the encrypted store. A critical step needs its own
typed phrase, and without it nothing is sealed and the step is shown with
what it runs and why it's critical.

| Option | |
| --- | --- |
| `--out FILE` | where to write the snapshot |
| `--confirm STEP=PHRASE` | approve a critical step with its phrase; repeatable |
| `--force` | replace an existing file at `--out` |
| `--no-dry-run` | skip `-WhatIf` dry runs in the validation before sealing |

### `keyjutsu plan stage FILE`

Downloads every artifact the plan needs, checks each against its pinned hash
and keeps it for the run. Nothing is downloaded while a plan runs.
`--pin FILE` writes a copy of the plan with the hash of each unpinned
artifact filled in, for you to review before approving.

### `keyjutsu plan verify SNAPSHOT`

Checks a snapshot hasn't been altered. `--environment` also compares this
machine with the one it was approved on.

### `keyjutsu plan hash FILE`

Prints each step's hash: what an approval of that step binds to.

### `keyjutsu plan diff OLD NEW`

What changed between two versions of a plan, and which steps that sends back
to validation.

## Running

### `keyjutsu run SNAPSHOT`

Runs an approved snapshot in this console. Refuses a snapshot this Windows
account didn't approve on this machine, one whose machine has changed since,
or one with a step that wasn't READY.

| Option | |
| --- | --- |
| `--mode performance\|assisted\|auto\|direct` | for steps without their own mode; the plan's default otherwise |
| `--clean` | skip your profile, history predictions and history saving |
| `--resume [CHECKPOINT]` | continue from the checkpoint beside the snapshot, or the one given (such as the previous snapshot's, after a revision) |
| `--settle STEP=succeeded\|failed` | settle a step left in doubt by a crash or a disarm |
| `--isolate worktree\|branch` | work in a new worktree, or on a new branch, rather than your checkout |
| `--ephemeral` | keep no record of the run in the history |

### `keyjutsu recover SNAPSHOT`

Shows how to undo what a stopped run changed, latest step first. Changes
nothing without `--confirm`.

| Option | |
| --- | --- |
| `--confirm` | carry the recovery out |
| `--from CHECKPOINT` | the run to recover; the checkpoint beside the snapshot otherwise |
| `--step STEP` | only these steps; repeatable |
| `--clean` | skip your profile for any recovery commands |

### `keyjutsu git diff SNAPSHOT`

What the run changed in each Git repository it worked in, and only that:
each file against how it was just before the run, with your own uncommitted
changes left out.

## History and Techniques

| Command | |
| --- | --- |
| `keyjutsu history list` | every recorded session, oldest first |
| `keyjutsu history show ID` | one session: task, agent, outcome and steps |
| `keyjutsu history recheck ID` | compare this machine with the one the session was approved on |
| `keyjutsu technique promote SESSION --name NAME` | make a completed session a Technique; `--param NAME=VALUE` makes a value a parameter, `--description` says what it's for |
| `keyjutsu technique list` | every Technique, with its revision and parameters |
| `keyjutsu technique use ID --out FILE` | a draft plan from a Technique, to validate and approve; `--param NAME=VALUE` for each value |
| `keyjutsu technique revise ID --template FILE` | save an adapted plan as a new revision; earlier ones are kept |
| `keyjutsu technique export ID --out FILE` | for sharing, without this machine's details |
| `keyjutsu technique import FILE` | read a shared Technique, as an untrusted draft |

## Setup and storage

| Command | |
| --- | --- |
| `keyjutsu setup path add\|remove\|status` | the install folder on your PATH |
| `keyjutsu setup explorer add\|remove\|status` | "Open KeyJutsu here" on Explorer's folder menus |
| `keyjutsu store clear` | delete what KeyJutsu keeps: `--history`, `--techniques`, `--artifacts`; nothing without naming it |

Both `setup` commands change only your own Windows account. `store clear`
leaves the records of approvals and checkpoints, which hold only hashes, so
snapshots you've approved still run.
