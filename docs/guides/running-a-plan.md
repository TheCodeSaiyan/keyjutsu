# Running a plan

**At the end:** an approved plan has run in a real shell, typed by your
keystrokes, each step checked after it ran, and you have a record of what it
did.

## Before you start

- An approved snapshot. [Getting a plan from an agent](planning.md) ends with
  one; [the example plan](../examples/check-a-service.json) makes a quick
  first one:

  ```powershell
  keyjutsu plan approve docs/examples/check-a-service.json --out check-a-service.approved.json
  ```

- If the plan works in a Git repository, a moment to decide whether it should
  work on your checkout or apart from it (step 2).

## Steps

### 1. Start it

```powershell
keyjutsu run check-a-service.approved.json
```

In the desktop app, once the plan is approved, **Arm KeyJutsu** takes its
place and the terminal fills the window.

Before anything is typed, KeyJutsu checks three things, and any one of them
stops the run with nothing done:

- **This account approved this snapshot, on this machine.** The approval
  recorded in the encrypted store has to be there; a snapshot that was edited
  and re-sealed, or copied from someone else, is refused.
- **The machine hasn't changed underneath it.** The environment is looked at
  again and compared with the one the plan was approved on. If a tool the
  plan relies on has changed version, KeyJutsu prints each change, before
  and after, and names the steps that depend on it; the run doesn't start
  until you validate and approve them again.
- **Every step was READY when it was approved**, and every download it needs
  is staged and still matches its hash.

A critical step approved more than an hour before it's reached is confirmed
again with its typed phrase, just before it runs: in the app's critical
dialog, or in the run's own console, where the keys you type while it asks
go to the answer and never to the shell. The hour counts to the moment the
step comes up, so a long run can't carry an approval past it. Within the
hour, the phrase you typed at approval stands, because being asked twice in
five minutes teaches you to type it without reading.

If the plan has Administrator steps, Windows asks once, now, to start
KeyJutsu's elevation broker, so no UAC prompt can appear in the middle of the
performance.

### 2. Decide where it works, if it's a Git repository

```powershell
keyjutsu run fix.approved.json --isolate worktree
keyjutsu run fix.approved.json --isolate branch
```

`worktree` works in a new worktree on a new local branch, leaving your
checkout untouched. `branch` switches to a new branch in place and brings your
uncommitted changes along. Without either, it works where you are, and
KeyJutsu records every changed file first, so afterwards it can tell its
changes from yours.

### 3. Perform it

Mash keys, as in [the demo](first-performance.md). Each step is typed, run,
and then checked: its internal checks run quietly, its visible checks are
typed like any other command, and the next step starts only when the shell
reports the last one finished and every check passed.

Some steps hand the keyboard back to you:

- **A credential.** The window's title says "Credential required", and the
  shell shows its own masked prompt. Stop mashing, press Enter, and type the
  secret yourself. KeyJutsu never types it and never stores it:
  it goes straight into PowerShell's `Read-Host -AsSecureString`, and keys you
  press after your answer are held so they can't spill into the next prompt.
  The variable is removed when the plan completes or fails; after a disarm
  KeyJutsu types nothing more, so it lasts until you close that shell.
- **Something only you can do,** like clicking through an installer. The
  window's title names the step, and it waits for your Enter.
- **A critical step, in the desktop app.** If its approval is more than an
  hour old, a dialog shows what it runs, what it affects and how it would be
  undone, and **Run critical step** only unlocks with the typed phrase.
  **Stop here** ends the run before it.

Ctrl+Alt+Shift+K disarms at any point and hands the terminal back. What had
finished counts; a step that was part-way through is recorded as in doubt,
because its effect is unknown.

### 4. Leave, and read the result

After the last step the keyboard is still held. Disarm with Ctrl+Alt+Shift+K,
then type `exit`. KeyJutsu then prints what happened:

```text
Recorded as session 20260925-184553-a1b2 in the encrypted history.
  ok     look
  ok     when
Complete: 2 steps.
```

A step that failed says what was expected and what actually happened, and
the plan stops there: nothing after it runs, and nothing is rolled back
without your say-so. [When a run stops](when-a-run-stops.md) takes it from
there.

If the plan worked in a Git repository, the summary also lists which files
KeyJutsu changed and which of your own changes it left alone:

```powershell
keyjutsu git diff fix.approved.json
```

shows KeyJutsu's changes alone, against each file as it was just before the
run.

`--ephemeral` keeps the run out of the history, and `--clean` starts the
shell without your profile.

## What gets written

- `check-a-service.approved.checkpoint.json`, next to the snapshot, before
  and after every step. It's what `--resume` and `keyjutsu recover` work
  from, and its hash is recorded each time, so an edited one is refused.
- A record in the encrypted history, unless `--ephemeral`.
- Recovery captures, beside the checkpoint, for steps that declared what
  they'd change: copies taken just before the step ran.

Next: [when a run stops](when-a-run-stops.md).
