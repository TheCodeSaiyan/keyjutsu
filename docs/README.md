# KeyJutsu documentation

The [README](../README.md) says what KeyJutsu does for you and how to get it.
This is everything behind it, in four groups. Guides walk you through one job
each. Plain-language pages cover the same jobs for people new to AI and to
terminals. Reference pages have every option. Internals are for people
changing KeyJutsu itself.

## Guides

Step by step, one job each: numbered steps, the exact command, and what you
should see.

- [Installing](guides/installing.md) — install, check the machine, and what the installer changes
- [Your first performance](guides/first-performance.md) — the safe demo, the keys, and getting the terminal back
- [Getting a plan from an agent](guides/planning.md) — ask, read what comes back, edit, validate, approve
- [Running a plan](guides/running-a-plan.md) — arm it, perform it, and what it asks you along the way
- [When a run stops](guides/when-a-run-stops.md) — a failed step, a restart in the middle, recovering, resuming
- [Reusing a plan that worked](guides/techniques.md) — the history, Techniques, and sharing one

## Plain language

For people new to AI or to terminals. Short sentences, one idea per step, and
every term explained the first time it appears.

- [KeyJutsu in plain words](plain/README.md) — what it is, what it isn't, and what you need first
- [Installing KeyJutsu](plain/installing.md) — from downloading to seeing it work
- [Your first performance](plain/first-performance.md) — mashing keys, safely
- [Running a plan safely](plain/running-a-plan.md) — what the checks are for, and when to say no

## Reference

- [What you get](features.md) — every part of KeyJutsu, with the commands
- [Recipes](recipes.md) — worked answers to the common jobs
- [Commands](commands.md) — the whole `keyjutsu` command surface
- [Performing](performing.md) — the modes, the keys, and what can never reach the shell
- [Agents](agent-integrations/README.md) — which agents work, how each is kept read-only, and what was verified
- [The plan format](schemas/plan.md) — what a plan is, who writes which part, and what is refused
- [Threat model](../THREAT_MODEL.md) — what is protected, from whom, and what isn't covered
- [Privacy](../PRIVACY.md) — what is stored, what is sent, and what is read

## Internals

- [Architecture overview](architecture/overview.md) — one Rust core, two front ends, and why
- [Execution state machine](architecture/execution-state-machine.md) — how a performance moves between states
- [Decisions](architecture/adr/README.md) — the architecture decision records
- [Colour and design](brand/colour-system.md) — the design kit, and how its colours reach the app
- [Contributing](../CONTRIBUTING.md) — building, testing, and the checks a change has to pass
