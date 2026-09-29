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
and you approve it. Only after that can anything be typed. If the agent is wrong, or something it read misled it, the worst it can
produce is a plan you can turn down.

## Steps

### 1. Describe the task

In the desktop app, **New task** asks "What do you want KeyJutsu to do?".
Write it, choose the agent, and add any context with **+ Add context**: an
error message, a log excerpt. Then **Plan task →**. **Open plan file…** opens
a plan you already have.

![The New task screen. "What do you want KeyJutsu to do?", a box for the task, + Add context, Open plan file…, the agent to ask and Plan task. Below, what this machine has: the agents, the terminal and its shell, how Administrator steps are handled, and no telemetry.](../images/new-task.png)

From the CLI:

```powershell
keyjutsu plan propose "Find out why the Print Spooler keeps stopping" --agent claude --out spooler.json
```

Without `--send`, nothing leaves the machine. You see exactly what would go:

```text
Request for Claude Code 2.1.282, in its read-only mode:
  --permission-mode plan: plan mode, no edits or commands
  text     this machine (761 characters)

Nothing was sent. Add --send to send this request.
```

The version shown is whichever one you have installed.

**This machine** is what KeyJutsu tells the agent about where the plan will
run, so it writes commands for what's really here rather than guessing: the
Windows build, each shell's version, whether KeyJutsu has Administrator
rights, where your Desktop, Documents and Downloads really are (a Desktop in
OneDrive included), and which common tools are on your PATH, with versions.
It names no user: folders are given as `%USERPROFILE%\…`. It goes with the
first proposal, from the app and the CLI alike.

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
readiness, its risk, and how it would be undone. What needs you comes first;
the checks that passed are folded into one line, each shown once, since a step
with many commands passes the same check many times:

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

Or ask the agent. Under **What needs you**, each step and the plan have a
reply box: **Send to agent** passes what you wrote as guidance, for that step
or for the whole plan (Ctrl+Enter does the same), and **Keep as a note** keeps
it for yourself. What the agent sends back arrives unvalidated and unapproved,
like any change.

You don't have to wait for the agent to finish. While it's working, the button
reads **Queue for the agent**: what you write waits under the reply box, in the
order the agent will get it, and goes when the agent is free. Replies in a row
about the same thing go as one request, so three quick thoughts cost one
revision, not three. **Remove** takes one back before it's sent. If a send
fails, nothing is lost: the reply goes back to the queue, the rest wait behind
it, and **Send again** carries on. A reply about a step the agent has since
removed goes to the plan, naming the step. The queue belongs to the plan it
was written for: starting or opening another plan clears it.

An agent can take minutes, so the line at the top of the window shows how long
it has been working, and **Stop** ends it: the agent and everything it started
are stopped, and nothing is changed, whether it was writing a new plan or
revising this one. Replies queued behind it are kept, held until you **Send
again**. KeyJutsu stops an agent itself after ten minutes, and says so.

From the CLI:

```powershell
keyjutsu plan revise spooler.json --step restart --guidance "Don't restart it; find out what stops it" --agent claude --out spooler.v2.json --send
keyjutsu plan diff spooler.json spooler.v2.json
```

`plan diff` says which steps changed and which need validating again.

For a second opinion, **Ask for review** in the app, or:

```powershell
keyjutsu plan review spooler.json --agent codex --send
```

When the agent can't find something out for itself (which of two folders,
whether to keep a file), it asks rather than guesses. Its questions appear
under **What needs you**, each option a button, with **In my own words** where
the agent allows it and **Carry on as planned** to leave the plan as it is.
The option the plan already follows is marked **(as planned)**: choosing it
just closes the question. Any other answer goes back to the agent, and what
it sends back is checked like any change. From the CLI, `keyjutsu plan answer spooler.json` lists them. An
unanswered question doesn't stop you approving the plan.

The reviewer can't change the plan. Its concerns appear under **What needs
you**, beside the steps they're about, each with **Ask the agent to address
it**, **It's fine** or **Edit the step**. **It's fine** dismisses it at once;
**It's fine, because…** lets you say why. Either way the concern, and your
reason if you gave one, is kept in the plan's history, so a saved plan shows
what was decided.

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
still critical. So is a command that deletes or changes things through a
wildcard (`Remove-Item C:\logs\*.log`, `del *.tmp`) or whatever a pipeline
hands it (`Get-ChildItem … | Remove-Item`): what it touches is decided when
it runs, not when you read it. The dry run shows what the wildcard matches
now; naming each file with `-LiteralPath` keeps the step to exactly those.

If the agent rated a step lower than KeyJutsu does, KeyJutsu's rating is the
one that counts, and the step says so. Where KeyJutsu rates it high or
critical, the step waits for your review; **Use KeyJutsu's rating**, under
**What needs you**, accepts it, or **Ask the agent why**.

Validation looks at the machine as it is now, before anything has run. A step
that needs a file an earlier step makes (step one prints `Cat.pdf`, step two
opens it) would fail that check, so when the earlier step declares it creates
the file, the check is left until just before the step runs, and the step
says so. Every step's preconditions are checked again at that moment, so one
that no longer holds stops the run before the step starts.

### 5. Approve it

**Approve plan** in the app, or:

```powershell
keyjutsu plan approve docs/examples/check-a-service.json --out check-a-service.approved.json
```

```text
Sealed 2 steps into check-a-service.approved.json
Snapshot f2aefa92427f82319b5025808bf5f943a55013a1b30a37ddb7b204dcc4edb18d
```

Approving validates once more, then seals the plan into a snapshot: every
step, bound to a hash of what it runs and everything before it. Change any
step afterwards and its approval, and every later step's, no longer applies.
The approval is also recorded in KeyJutsu's encrypted store, so a snapshot
that was edited, or approved on another machine or account, is refused when
you try to run it.

A **critical** step is never covered by approving the plan. It needs its own
typed phrase, and KeyJutsu shows you what it runs and why it's critical
first. For [a plan that deletes a build cache](../examples/clear-build-cache.json):

```text
Not sealed: these critical steps need their own typed confirmation.

  CRITICAL ACTION  clear-cache: Delete the old build cache
    runs:          Remove-Item -Recurse -Force -LiteralPath C:/Users/Public/BuildCache
    target:        C:/Users/Public/BuildCache (FileDeleted)
    impact:        Deletes the whole folder; it cannot be brought back, only rebuilt.
    why critical:  Remove-Item -Recurse deletes a whole tree
    why critical:  irreversibly deletes C:/Users/Public/BuildCache
    why critical:  declared irreversible
    dry run:       `Remove-Item -Recurse -Force -LiteralPath C:/Users/Public/BuildCache` -WhatIf: What if: Performing the operation "Remove Directory" on target "C:\Users\Public\BuildCache".
    reversibility: None: The cache is rebuilt on the next build.
    recovery:      none stated
    confirm: --confirm clear-cache="DELETE THE OLD BUILD CACHE"
```

In the app the same appears as a dialog, with **Approve critical step**
enabled only once the phrase is typed. The phrase is words, not a key, so no
amount of mashing can produce it.

![The critical action dialog for a step that deletes C:/Users/Public/BuildCache. It shows the target, the exact command, the impact, that recovery is none, and the validation evidence including the -WhatIf result. At the foot: to approve this step, type DELETE THE OLD BUILD CACHE, with Approve critical step disabled until it's typed.](../images/critical-step.png)

### 6. Check the snapshot, whenever you like

```powershell
keyjutsu plan verify check-a-service.approved.json --environment
```

```text
Intact: 2 steps approved, sealed 2026-09-25T20:24:37Z
Snapshot f2aefa92427f82319b5025808bf5f943a55013a1b30a37ddb7b204dcc4edb18d
Environment unchanged since approval.
```

## If a step won't go READY

Everything that stops a step is listed under **What needs you** in the step,
with the answers that fit it. KeyJutsu chooses those answers from what kind of
finding it is, never from anything the agent wrote, and each one is something
you could already do by hand: stage, edit, remove, validate again, or send the
agent guidance written from the finding.

- **INVALID**: a mistyped command or parameter, or broken syntax. The finding
  names it. Edit the step, or retry it with the agent.
- **BLOCKED**: something it needs isn't here: a tool, a version, a service.
  Install it and choose **I've changed the machine: validate again**, or
  **Ask for a step without it**.
- **REVIEW**: KeyJutsu couldn't prove enough, or KeyJutsu rates the step high
  or critical and the agent rated it lower. Read the step; if it's right,
  **Use KeyJutsu's rating**, edit it to say so honestly, or **Run it as it
  is**: the step is ready on your word, with what validation found still
  shown beside it and your decision kept in the plan's history. It holds when
  you validate again while the findings are the same, and goes if the step
  changes. A critical step still needs its phrase, and a BLOCKED or INVALID
  step can't be run as it is: those can't work here.
- **A step needs a download.** **Stage the download** in the step, or
  `keyjutsu plan stage spooler.json`, fetches each file and checks it against
  its pinned hash before approval. Nothing is downloaded while a plan runs.
- **Still stuck?** **Save plan** writes the plan, with what validation found
  for every step, to a file you can keep, open again with **Open plan file…**,
  or hand to someone helping. It holds the plan's commands and paths, so read
  it before you send it.

Next: [running a plan](running-a-plan.md).
