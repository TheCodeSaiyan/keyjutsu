# Performing

A performance is KeyJutsu typing an approved command into a real shell, one
keystroke of yours at a time. This page has every mode and key, and the rules
about what can reach the shell, which hold in every mode.

## The modes

| Mode | Each key you press | Submits | For |
| --- | --- | --- | --- |
| `performance` | types the next character | per `--submit` | the show: the default |
| `assisted` | types a short burst, up to the end of a word | per `--submit` | long commands, fewer keys |
| `auto` | nothing: it types and runs everything by itself | by itself | watching rather than playing |
| `direct` | nothing: each command is sent whole, with no typing effect | by itself | long or quiet steps |

`--turbo` changes `performance` to a word per key rather than a character.
A plan can set its own default mode and a step can override it, so a
long-running step can go `direct` in the middle of a performance. `--mode` on
`keyjutsu run` sets the mode for steps that don't set their own.

`auto` has no authority the other modes lack. It runs the same approved
snapshot through the same checks; only who presses the keys changes.

A credential step, or a step that asks you to do something yourself, is never
performed in any mode: staged typing turns off, and your keys go to the shell
for real.

## Submitting

`--submit` decides what happens once a command is completely typed:

- `any-key`, the default: the next key you press submits it.
- `enter`: only a real Enter submits, so you can read it first.
- `auto`: it submits as soon as the last character is typed.

An incomplete command is never submitted, whatever you press. Enter halfway
through a command is just another key.

## The keys

| Key | Does |
| --- | --- |
| Any ordinary key | advances the command, in `performance` and `assisted` |
| Ctrl+C | a real interrupt, always, to whatever is running |
| Esc | goes to a running command, for programs that use it; ignored while KeyJutsu is typing |
| Ctrl+Shift+K | CLI: pause and resume. Desktop: the operator controls |
| Ctrl+Alt+Shift+K | disarm at once and hand the terminal back |

**The operator controls** in the desktop app are a small panel in the corner,
for you rather than anyone watching: the current step, its state, how much is
typed, the next step, progress, and **Resume** and **Disarm**. Opening them
pauses the performance.

![A performance, paused. The terminal shows Get-ComputerInfo -Property OsNa, typed so far by 31 keys. In the corner, the operator controls: step 01, Describe this computer, paused, 31 of 89 characters typed, next Show the PowerShell version, 0 of 3 done, with Resume and Disarm.](images/performance.png)

**The disarm chord** works in every state. It's recognised before anything
else looks at the key, and it erases whatever was half-typed, so the line you
get back is empty. Neither the CLI nor the app offers a setting to change it
yet; the engine underneath already refuses any binding without Ctrl, Alt or
the Windows key, because ordinary typing could trigger it. Esc isn't the
disarm on purpose: people press it by reflex.

## What can never reach the shell

These hold in every mode, and each has a test that fails without it
([threat model](../THREAT_MODEL.md#invariants)):

- **Only the staged text**, while a performance owns the keyboard. The key you
  pressed decides when a character appears, never which one.
- **Nothing while a command runs**, apart from Ctrl+C and Esc. Keys pressed
  then are swallowed, so they can't land in the command's own input.
- **Nothing between steps.** Keys pressed while KeyJutsu waits for the next
  step are held, not typed.
- **No half-typed command.** Only a finished command can be submitted.
- **No performance on a busy or dirty line.** Arming is refused while
  something is running or you've typed something the performance would add to.
- **Nothing from the window into an armed line.** The desktop's terminal
  can't type into a line a performance owns.

A step moves on because the shell reports it finished, through a mark its
prompt prints, never because time passed. The mark carries a secret per
session, so a command printing a fake one is shown as output and ignored.

## Your profile

By default a performance uses your own PowerShell profile: your prompt, your
aliases, your history. `--clean` (or **Clean** in the app) starts the shell
without it, turns PSReadLine's predictions off and saves no history. Use it
when people are watching: with your own profile, PSReadLine can show earlier
history entries as grey suggestions while KeyJutsu types, and the performed
commands are saved to your history as if you'd typed them
([ADR 0009](architecture/adr/0009-clean-profile-hides-history.md)).

## Shells

PowerShell 7 if it's installed, otherwise Windows PowerShell 5.1, which every
Windows has; `--shell` picks one. `cmd` works too, with one limit: it can't
report an exit code, so a step there is "finished, success unverified" rather
than "succeeded". Credential steps are refused in `cmd`, because it has no
masked prompt.
