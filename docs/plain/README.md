# KeyJutsu in plain words

This page is for you if you're new to AI, or to typing commands. It explains
what KeyJutsu is before it asks you to do anything.

## What a terminal is

A **terminal** is a window where you type instructions to the computer,
instead of clicking. Each instruction is called a **command**. Windows comes
with one called **PowerShell**.

One short command can change settings or delete files. So it matters which
commands run.

## What an AI agent is

An **AI agent** is a program you talk to in ordinary words. You say what you
want, and it looks around your computer and works out what to do. Some
well-known ones are **Claude Code**, **Codex**, **Gemini** and **GitHub
Copilot**.

Three things are worth knowing about any agent:

1. **It can be wrong and sound sure.** It sounds the same either way. So its
   work gets checked.
2. **It can be misled.** If it reads a web page or a file with bad
   instructions hidden in it, it may follow them.
3. **It only knows what it was told,** and what it went and looked at.

## What KeyJutsu is

KeyJutsu sits between the agent and your computer.

- The agent **suggests** a list of commands. That list is called a **plan**.
  The agent isn't allowed to run anything itself.
- KeyJutsu **checks** the plan against your computer: does each command
  exist, is anything missing, how risky is it.
- **You** read the plan and say yes or no. Saying yes is called
  **approving**.
- Only then does anything run.

And the fun part: when it runs, you **mash your keyboard**, any keys at all,
and the approved commands type themselves out perfectly, one letter per key,
and run for real. It looks like expert typing. The typing is theatre. The
commands are real.

## What it won't do

- It won't run anything you haven't approved.
- It won't type a password for you. When a step needs one, you type it
  yourself, into a box that hides it.
- It won't run a very risky step, like deleting a whole folder, unless you've
  typed a sentence to confirm that exact step.
- It won't send anything about you anywhere. It has no telemetry. The agent
  talks to its own company under your own account; KeyJutsu doesn't.

## What you need before you start

- **A computer running Windows 11.**
- **About ten minutes.**
- **An AI agent, later.** You don't need one to try KeyJutsu: it has a safe
  demo. To get plans from an agent, you'll need one installed and signed in.
  The [agents page](../agent-integrations/README.md) says which work.

## Where to go next

Take these in order:

1. [Installing KeyJutsu](installing.md)
2. [Your first performance](first-performance.md)
3. [Running a plan safely](running-a-plan.md), when you're ready for a real
   job

If something goes wrong at any point, open a terminal, type
`keyjutsu doctor` and press Enter. It tells you what's missing.
