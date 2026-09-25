# Recipes

The jobs people turn up wanting to do. Every command here is real; the page
each section points at has the detail.

## Show someone what it does, safely

```powershell
keyjutsu demo --clean
```

Three read-only commands, typed by whatever keys you mash. `--clean` keeps
your prompt theme and your history's grey suggestions off the screen, which
matters when someone's watching. Ctrl+Alt+Shift+K gives the terminal back.
[Your first performance](guides/first-performance.md)

## Check a machine before relying on it

```powershell
keyjutsu doctor
keyjutsu agents
```

`doctor` exits 1 if anything fails, so it works in a script. Neither sends
anything anywhere.

## Get a plan for a problem, without anything changing

```powershell
keyjutsu plan propose "Docker Desktop won't start after the last update" --agent codex --out docker.json
keyjutsu plan propose "Docker Desktop won't start after the last update" --agent codex --out docker.json --send
keyjutsu plan validate docker.json
```

The first shows what would be sent and stops. The agent runs read-only, and
validation reads commands without running them, so at the end of this nothing
on the machine has changed. [Getting a plan](guides/planning.md)

## Give the agent a log to work from

```powershell
keyjutsu plan propose "Why does this build fail?" --agent claude --file build.log --out build.json
```

The file is redacted before it's sent; the preview says how many of each kind
of secret were taken out.

## Get a second opinion on a plan

```powershell
keyjutsu plan review docker.json --agent claude --send
```

A different agent is less likely to share the first one's blind spots.
It can't change the plan.

## Run a plan

```powershell
keyjutsu plan approve docker.json --out docker.approved.json
keyjutsu run docker.approved.json
```

[Running a plan](guides/running-a-plan.md)

## Run it without anyone mashing

```powershell
keyjutsu run docker.approved.json --mode direct
```

Same snapshot, same checks, no typing effect. `--mode auto` types it by
itself instead, if you want to watch.

## Approve a critical step

```powershell
keyjutsu plan approve docs/examples/clear-build-cache.json --out clear-build-cache.approved.json --confirm clear-cache="DELETE THE OLD BUILD CACHE"
```

Run `plan approve` without `--confirm` first: it prints each critical step,
what it runs, why it's critical and the exact phrase to type.

## Keep a run off your working tree

```powershell
keyjutsu run fix.approved.json --isolate worktree
keyjutsu git diff fix.approved.json
```

## Undo what a failed run did

```powershell
keyjutsu recover fix.approved.json
keyjutsu recover fix.approved.json --confirm
```

The first only shows the recovery plan. [When a run stops](guides/when-a-run-stops.md)

## Fix the plan and carry on from where it stopped

```powershell
keyjutsu plan revise fix.json --step configure --guidance "Keep the existing proxy setting" --agent claude --out fix.v2.json --send
keyjutsu plan diff fix.json fix.v2.json
keyjutsu plan approve fix.v2.json --out fix.v2.approved.json
keyjutsu run fix.v2.approved.json --resume fix.approved.checkpoint.json
```

Steps that succeeded and didn't change aren't run again.

## Carry on after a restart

```powershell
keyjutsu run upgrade.approved.json --resume
```

You'll be asked to type `RESUME`. KeyJutsu checks the restart happened before
it continues.

## Check a snapshot hasn't been touched

```powershell
keyjutsu plan verify fix.approved.json --environment
```

## Do the same job again next month

```powershell
keyjutsu history list
keyjutsu technique promote 20260925-184553-a1b2 --name "Reset the Docker network" --param adapter=vEthernet
keyjutsu technique use reset-the-docker-network --param adapter=Ethernet --out reset.json
keyjutsu plan validate reset.json
```

[Reusing a plan that worked](guides/techniques.md)

## Take KeyJutsu off the PATH, or out of Explorer

```powershell
keyjutsu setup path remove
keyjutsu setup explorer remove
```

## Forget everything KeyJutsu kept

```powershell
keyjutsu store clear --history --techniques --artifacts
```
