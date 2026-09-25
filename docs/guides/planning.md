# Getting a plan from an agent

**At the end:** an agent has investigated your task without changing
anything, KeyJutsu has checked its plan against this machine, you've read
every step, and the plan is approved and sealed so that it, and only it, can
run.

## Before you start

- An agent installed and signed in: Codex, Claude Code, Gemini, GitHub
  Copilot or Cursor. `keyjutsu agents` says which you have.
  [Agents](../agent-integrations/README.md) says which have been verified and
  how each is kept read-only.
- A task you could describe to a colleague in a sentence or two.

No agent? You can still write a plan by hand, or open one someone sent you;
start at step 3.

## Why it works this way

The agent proposes and nothing more. It runs in its own read-only mode, so it
can look but not change anything, and what it hands back is data, not
commands to run. KeyJutsu then checks the plan itself, against this machine,
and you approve it. Only after that can anything be typed. An agent that is
wrong, careless or misled by something it read gets as far as a plan you
decline.

## Steps

### 1. Describe the task

In the desktop app, **New task** asks "What do you want KeyJutsu to do?".
Write it, choose the **Primary agent**, and add any context with
**+ Add context**: an error message, a log excerpt. Then **Plan task →**.

From the CLI:

```powershell
keyjutsu plan propose "Find out why the Print Spooler keeps stopping" --agent claude --out spooler.json
```

Without `--send`, nothing leaves the machine. You see exactly what would go:

```text
Request for Claude Code 2.1.282, in its read-only mode:
  --permission-mode plan: plan mode, no edits or commands
  context: none beyond the task

Nothing was sent. Add --send to send this request.
```

Add `--file` for a file to include and `--folder` for a folder the agent may
investigate. Anything that looks like a secret, such as a token, a key or a
`password=` line, is redacted before sending, and the preview says how many
of each were taken out. Secrets that don't look like one aren't caught, and a
folder is read by the agent itself, so the preview warns about files there
with names like `.env` or `*.pem`.

When it looks right, send it:

```powershell
keyjutsu plan propose "Find out why the Print Spooler keeps stopping" --agent claude --out spooler.json --send
```

The agent investigates, then answers with a plan. If the answer isn't a
valid plan, KeyJutsu tells the agent what was wrong and asks again. It ends
with "Proposed N steps in M attempt(s)", and the plan is written to
`spooler.json`.

### 2. Read the plan

In the desktop app, the **Plan** space lists the steps in the middle. Select
one to see its commands, what it's meant to achieve, the evidence behind its
readiness, its risk, and how it would be undone:

![The KeyJutsu Plan screen. Two steps of the example plan, both READY and low risk. The first is selected, showing its command, Get-Service -Name Winmgmt, the evidence behind its readiness, and Edit, Retry step with agent, Move and Remove. At the foot, 2/2 ready, no elevation, and Validate and Approve plan.](../images/plan-workspace.png)

From the CLI:

```powershell
keyjutsu plan check spooler.json
```

That checks the plan against the schema and its structure, then shows the
order the steps would run in:

```text
Check that Windows Management Instrumentation is running: valid (2 steps)

   1. look: Look at the Winmgmt service
   2. when: Say when Windows last started
```

This output, and the rest on this page, is from
[the example plan](../examples/check-a-service.json), which you can try
without an agent.

### 3. Change what you don't like

In the desktop app, select a step and choose **Edit**, **Move up**,
**Move down** or **Remove**, or **+ Add a step**. Every edit is checked as a
whole plan in Rust before it's kept, and an edit the plan's rules would refuse
leaves it as it was.

Or ask the agent: **Retry step with agent** with a sentence of guidance, or
**Ask agent to reconsider the plan** for the whole thing. From the CLI:

```powershell
keyjutsu plan revise spooler.json --step restart --guidance "Don't restart it; find out what stops it" --agent claude --out spooler.v2.json --send
keyjutsu plan diff spooler.json spooler.v2.json
```

`plan diff` says which steps changed and which need validating again.

For a second opinion, **Ask for review** in the app, or:

```powershell
keyjutsu plan review spooler.json --agent codex --send
```

The reviewer can't change the plan. Its findings are shown beside the steps
they're about.

### 4. Validate it

**Validate** in the app, or:

```powershell
keyjutsu plan validate docs/examples/check-a-service.json
```

```text
  READY      High    risk Low      look
      ? Checked without your PowerShell profile: aliases and functions it defines were not considered
  READY      High    risk Low      when
      ? Checked without your PowerShell profile: aliases and functions it defines were not considered

All 2 steps are ready.
```

Validation reads each command without running it: its syntax, whether each
command and parameter exists here, the tools and versions it needs, what it
assumes, the privilege it needs, and how risky it is. Nothing a plan names is
run. The one exception is `-WhatIf`, for built-in management commands whose
arguments are plain values, because there PowerShell itself reports what
would happen; `--no-dry-run` skips even that.

Each step comes back READY, REVIEW, BLOCKED or INVALID, with how well
that was proved. The risk is KeyJutsu's own view: an agent can raise a
step's risk but never lower it, so a recursive delete labelled "low" is
still critical.

### 5. Approve it

**Approve plan** in the app, or:

```powershell
keyjutsu plan approve docs/examples/check-a-service.json --out check-a-service.approved.json
```

```text
Sealed 2 steps into check-a-service.approved.json
Snapshot 957593ac4bf41d5f4f763e9f721d40d7220e6496da0db73d6b597c467203c189
```

Approving validates once more, then seals the plan into a snapshot: every
step, bound to a hash of what it runs and everything before it. Change any
step afterwards and its approval, and every later step's, no longer applies.
The approval is also recorded in KeyJutsu's encrypted store, so a snapshot
that was edited, or approved on another machine or account, is refused when
you try to run it.

A **critical** step is never covered by approving the plan. It needs its own
typed phrase, and KeyJutsu shows you what it runs and why it's critical
first:

```text
Not sealed: these critical steps need their own typed confirmation.

  CRITICAL ACTION  remove-data: Remove local Docker data
    runs:          Remove-Item -Recurse -Force -LiteralPath C:/ProgramData/Docker/data
    target:        C:/ProgramData/Docker/data (FileDeleted)
    impact:        Deletes every local container, image and volume.
    why critical:  Remove-Item -Recurse deletes a whole tree
    why critical:  runs as Administrator
    why critical:  irreversibly deletes C:/ProgramData/Docker/data
    why critical:  declared irreversible
    reversibility: None: Images can be pulled again; volumes are gone.
    recovery:      None
    confirm: --confirm remove-data="REMOVE LOCAL DOCKER DATA"
```

In the app the same appears as a dialog, with **Approve critical step**
enabled only once the phrase is typed. The phrase is words, not a key, so no
amount of mashing can produce it.

### 6. Check the snapshot, whenever you like

```powershell
keyjutsu plan verify check-a-service.approved.json --environment
```

```text
Intact: 2 steps approved, sealed 2026-09-25T18:45:53Z
Snapshot 957593ac4bf41d5f4f763e9f721d40d7220e6496da0db73d6b597c467203c189
Environment unchanged since approval.
```

## If a step won't go READY

- **INVALID**: a mistyped command or parameter, or broken syntax. The finding
  names it. Edit the step, or retry it with the agent.
- **BLOCKED**: something it needs isn't here: a tool, a version, a service.
  Install it, or ask the agent for a step that doesn't need it.
- **REVIEW**: KeyJutsu couldn't prove enough, or the agent's risk
  label was lower than KeyJutsu's. Read the step; if it's right, edit it to
  say so honestly.
- **A step needs a download.** **Stage downloads** in the app, or
  `keyjutsu plan stage spooler.json`, fetches each file and checks it against
  its pinned hash before approval. Nothing is downloaded while a plan runs.

Next: [running a plan](running-a-plan.md).
