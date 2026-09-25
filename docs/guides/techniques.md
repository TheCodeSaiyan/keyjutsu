# Reusing a plan that worked

**At the end:** a plan that worked has become a Technique, reused with new
values on a different day or machine, and checked again rather than trusted
because it worked last time.

## Before you start

- A run that completed, recorded in the history. Runs from the CLI are
  recorded unless you used `--ephemeral`. Runs from the desktop app aren't
  recorded yet ([D26](../architecture/deviations.md#d26-no-desktop-history-or-technique-screens-yet)),
  and it has no history or Technique screens, so this guide is CLI only.

## Why a Technique isn't a shortcut

A plan that worked on Tuesday on this machine says nothing certain about next
month, or about a machine with another version of the tool it drives. So a
Technique never runs because it worked before. Using one makes a **draft**
plan, which is validated and approved here, exactly like an agent's. What the
Technique adds is the record of where it worked, so KeyJutsu can say which
steps are on familiar ground and which aren't.

## Steps

### 1. Find the session

```powershell
keyjutsu history list
keyjutsu history show 20260925-184553-a1b2
```

`list` shows each session's id, outcome and task, oldest first. `show` gives
the task, the agent, the outcome, each step's result, and whether the approved
snapshot inside the record is still intact.

`keyjutsu history recheck` with the id compares this machine with the one the
session was approved on, and names the steps that would need validating again.

### 2. Make it a Technique

```powershell
keyjutsu technique promote 20260925-184553-a1b2 --name "Check a Windows service" --param service_name=Winmgmt
```

`--param` turns a value in the plan into a parameter: every place `Winmgmt`
appeared becomes `{{kj:service_name}}`, with `Winmgmt` as its default. A
parameter's value is data, never code. Whatever pattern it's given, a value
can't contain quotes, `$`, `;`, `&`, pipes, redirection, backticks, braces,
parentheses, line breaks or invisible characters, so it can't turn into a
second command.

### 3. Use it

```powershell
keyjutsu technique list
keyjutsu technique use check-a-windows-service --param service_name=Spooler --out spooler.json
```

`use` writes a draft and says how this machine compares with where the
Technique worked:

- "This machine matches an environment it worked on."
- "This machine differs from where it worked", followed by what differs and
  the steps that require revalidation.
- "It has no record of working on any machine KeyJutsu knows", for one you
  imported: every step must be validated.

It always ends "It is a draft: validate and approve it like any other plan."
Then do that, as in [getting a plan](planning.md):

```powershell
keyjutsu plan validate spooler.json
keyjutsu plan approve spooler.json --out spooler.approved.json
```

### 4. Adapt it

If this machine needs the plan changed, change the draft, then save it as a
new revision:

```powershell
keyjutsu technique revise check-a-windows-service --template spooler.json
```

Earlier revisions are kept, never rewritten, so the version that worked is
still there if the adaptation doesn't.

### 5. Share it

```powershell
keyjutsu technique export check-a-windows-service --out check-a-service.technique.json
keyjutsu technique import check-a-service.technique.json
```

An export leaves out the session it came from and the environments it worked
on, because those say more about your machine than you probably mean to
share. An import is treated as untrusted: its plan is checked against the
schema, anything in it claiming validation, approval or a known-good
environment is dropped, and it arrives with no record of working anywhere.
That was tested with a crafted export claiming all three, and one hiding a
right-to-left override in a command, which is refused outright.

## Clearing it

```powershell
keyjutsu store clear --history
keyjutsu store clear --techniques
```

Nothing is cleared without naming it. The history and Techniques are
encrypted with a key only your Windows account on this machine can unlock,
so a copied disk or another account can't read them.
