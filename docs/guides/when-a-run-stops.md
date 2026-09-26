# When a run stops

**At the end:** you know why the run stopped, and you've either put the
machine back as it was, repaired the plan and carried on, or crossed the
restart the plan was waiting for.

A plan stops for one of four reasons, and each has its own way on. In all four, nothing is rolled back, retried or resumed until you say so.

## A step failed

KeyJutsu prints what it expected and what it found. For a step whose check
wants the Print Spooler running, it ends like this:

```text
  FAILED restart-spooler
         service Spooler: expected running, got stopped

Step `restart-spooler` failed. The plan stopped there.
  expected: service Spooler
  actual:   expected running, got stopped

Nothing has been rolled back. Your choices:
  Diagnose first: the shell is as the step left it.
  Review the recovery plan: `keyjutsu recover fix.approved.json`
  Roll back: the same, with --confirm.
  Stop here without rolling back: do nothing.
```

The desktop app shows the same choices, with **Review recovery plan**.

### 1. Look first

The shell is exactly as the failing step left it, so the error, the service's
state or the half-written file is still there to see. A rollback straight away
would destroy the evidence of what went wrong.

### 2. See what undoing it would do

```powershell
keyjutsu recover fix.approved.json
```

This only shows the plan, latest step first, and changes nothing. For a plan
that rewrote a configuration file and then added a firewall rule, it reads:

```text
Recovery plan, latest step first:
  set-config: restore what was captured before it ran
      C:/ProgramData/App/config.json
  add-rule: run its approved recovery commands
      Remove-NetFirewallRule -Name AppInbound

Nothing has been changed. To carry this out, run the same command with --confirm.
```

A step is restored only from what it declared it would change, captured
just before it ran, so nothing it didn't declare is touched. Each capture's
hash is checked before it's used, and a copy that changed since is refused
rather than trusted. A step with no recovery says "cannot be recovered" and
why. `--step` limits recovery to the steps you name.

### 3. Roll back, if that's right

```powershell
keyjutsu recover fix.approved.json --confirm
```

In the app, **Roll back now**. Each step reports `recovered` or `FAILED`,
with the check that decided it.

### 4. Or repair and carry on

Fix the plan instead, with the agent reading what the step actually printed
rather than guessing. The failure message names the session the run was
recorded as; pass it with `--session`:

```powershell
keyjutsu plan revise fix.json --step restart-spooler --session 20260925-184553-a1b2 --guidance "Start its dependency first" --agent claude --out fix.v2.json --send
keyjutsu plan approve fix.v2.json --out fix.v2.approved.json
keyjutsu run fix.v2.approved.json --resume fix.approved.checkpoint.json
```

The agent is shown the step, your guidance, what validation found, and the
last 4,000 characters the step printed, with anything that looks like a
secret taken out first. It's told that the output is data from the machine
and not instructions, because command output is exactly where an instruction
meant for an agent would be planted. Without `--send` you see what would go.
A run made with `--ephemeral` has no recorded session, so there's nothing to
pass.

In the desktop app it's one panel: after a failure it shows what the step
printed, and **Ask the agent to fix it**, with a box for your guidance, sends
the same request. The fixed plan opens for you to validate and approve, and
**Arm KeyJutsu** then carries on from the step that failed.

Either way, steps that succeeded and haven't changed aren't run again. A
result only carries over where the step's hash is the same in the new
snapshot, so a step you changed, or one after it, runs again.

## You disarmed part-way

```text
Disarmed during `set-config`.
A command had been submitted, so its effect is unknown. Check, then resume with --settle.
```

If a command had already been submitted, KeyJutsu can't know whether it did
its work, so the step is in doubt and a resume won't start until you say
which it was. Look at the machine, then:

```powershell
keyjutsu run fix.approved.json --resume --settle set-config=succeeded
```

or `set-config=failed`, to run it again. The same happens after a crash or a
power cut: the checkpoint written before each step starts is what records
that it had begun.

## The plan waits for a restart

A plan can be written in phases with a boundary between them: a Windows
restart, a sign-out, a new shell, or a WSL or Docker restart. At the boundary
the run stops and tells you what to do:

```text
Phase `install` is done. The plan now waits for a Windows restart.
  Restart Windows when you are ready; KeyJutsu does not restart it for you.
Then: keyjutsu run fix.approved.json --resume
```

KeyJutsu doesn't restart your machine, and doesn't start itself after you
sign in: restarting someone's computer is about as disruptive as a tool gets, and resuming by itself would run the next phase before anyone looked.

In the desktop app, the run panel says what to do instead. After a Windows
restart or a sign-out, open KeyJutsu: a banner names the plan that's waiting,
with **Continue it**. That opens the plan as it was approved; open a terminal
and arm it as before. After a new shell or a WSL or Docker restart, the app
is still open, so arm the plan again once the boundary has happened.

Resuming checks, in this order:

1. **The boundary really happened.** It recorded what should change: the boot
   time, the logon id, the shell's process, WSL's boot id. Resuming before the
   restart is refused. Docker has nothing to check, so there it takes your
   word, and says so.
2. **The machine still matches the approved one.** A change that affects a
   remaining step stops the run and names the step.
3. **What the earlier phases did still holds.** Every check of every
   completed step runs again, and nothing is assumed to have survived the
   restart.

Then you're asked, with `RESUME` typed out. The CLI asks before the checks,
on the plain console. The app asks after them, and shows what they found:
whether it saw the boundary happen, anything about the machine that changed,
and each earlier check that still holds. A failed check stops the run before
you're asked. "Not now" stops it too, still waiting, so you can come back.

This has been done across a real Windows restart, from the CLI and from the
app. A sign-out has only been tested with its identity faked.

## It wouldn't start at all

`Could not continue: ...` names the reason: a snapshot this account didn't
approve, a machine that has changed, a download that no longer matches its
hash, a broker that didn't start, or a checkpoint that couldn't be written
before a step. In every one of those cases nothing ran. The reason says what
to do: usually validate and approve again.
