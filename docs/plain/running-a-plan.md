# Running a plan safely

**By the end of this page:** you know what happens between asking an AI for
help and anything changing on your computer, and when to say no.

This page is about understanding. The step-by-step version, with every
command, is [running a plan](../guides/running-a-plan.md).

## The five stages

### 1. You ask

You describe what you want in ordinary words. For example: "Find out why the
printer keeps stopping."

KeyJutsu first shows you **what it's about to send** to the AI agent. Nothing
goes until you say so. If what you pasted contains something that looks like
a password or a key, KeyJutsu takes it out first. A password that doesn't
look like one won't be caught, so don't paste one.

### 2. The agent investigates, and suggests

The agent looks around your computer. It's only allowed to **look**, not to
change anything. Then it suggests a **plan**: a list of steps, each with the
commands it would run and why.

### 3. KeyJutsu checks

KeyJutsu reads every command in the plan **without running it**. It checks:

- that each command really exists on your computer;
- that anything the command needs is installed;
- how risky each step is.

Each step gets a verdict. **READY** means it passed. Anything else says what's
wrong.

The AI's own opinion of how risky a step is doesn't count for much. If
KeyJutsu thinks a step is more dangerous than the AI said, KeyJutsu's view
wins.

### 4. You decide

Read each step. You're looking for three things:

- **Does it do what you asked?** Not more.
- **Does it change anything you didn't expect?** Deleting, restarting,
  uninstalling.
- **Can it be undone?** Each step says.

If a step is **critical**, which means it could do serious harm, KeyJutsu asks
you to **type a sentence** to confirm that one step. A key press isn't
enough. This is so you can't approve something dangerous by accident.

Saying yes to the plan is called **approving**. Once you approve, the plan is
**sealed**: nobody, not even the AI, can change it and still have it run.

### 5. It runs, and checks itself

You mash your keyboard, as in [the demo](first-performance.md), and the
approved commands type themselves and run.

After each step, KeyJutsu checks that it worked. If a step fails, **everything
stops**. Nothing else runs, and nothing is undone without you choosing to.

## When to say no

Say no, or ask the AI to try again, if:

- you don't understand what a step does;
- a step does more than you asked;
- a step deletes something and says it can't be undone;
- you're being hurried. KeyJutsu never needs you to decide quickly.

Saying no costs nothing. The plan just doesn't run.

## If you need to type a password

Some steps need a password. KeyJutsu **never types it for you**, and never
sees it. The terminal shows a box that hides what you type. Stop mashing,
press Enter, and type the password yourself.

## If it stops part-way

KeyJutsu tells you which step stopped and why. You can:

- look at what happened first;
- ask KeyJutsu to **undo** what the plan changed, after it shows you exactly
  what it would do;
- or leave things as they are.

[When a run stops](../guides/when-a-run-stops.md) has the details.
