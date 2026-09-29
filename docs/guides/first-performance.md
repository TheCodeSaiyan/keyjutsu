# Your first performance

**At the end:** you've mashed your keyboard while three real commands typed
themselves perfectly and ran, and you know how to get the terminal back at
any moment.

## Before you start

- KeyJutsu installed, and `keyjutsu doctor` showing `ok` for ConPTY and at
  least one shell. [Installing](installing.md) covers both.
- Nothing else. The demo needs no agent and no plan, and every command in it
  only reads.

## Steps

### 1. Start the demo

```powershell
keyjutsu demo --clean
```

A shell starts inside KeyJutsu's own terminal and draws its prompt. Nothing
is typed yet. `--clean` skips your PowerShell profile and history, so an
oh-my-posh prompt or PSReadLine's grey predictions can't get in the way the
first time; drop it once you've seen how it behaves.

In the desktop app, the first screen has **Try the safe demo**. It stages
the same three commands; open **Terminal**, choose the **Clean** profile, and
press **Arm KeyJutsu** to begin:

![The Terminal screen with the safe demo ready. On the left, the mode (Performance selected), Turbo, what submits a command, the demo's read-only commands and the red Arm KeyJutsu button, then Readiness, naming PowerShell 7 and Git as not installed. On the right, Windows PowerShell with the Clean profile, waiting at its prompt.](../images/terminal.png)

### 2. Mash the keyboard

Press any letters, as fast as you like. Each key types the next character of
the first command, whatever the key was:

```powershell
Get-ComputerInfo -Property OsName, OsVersion, OsArchitecture, CsNumberOfLogicalProcessors
```

When the command is complete, the next key submits it and it runs for real.
While it runs, your keys go nowhere: they are swallowed, so a stray letter
can't land in the command's input. When the shell's prompt comes back,
KeyJutsu moves to the next command:

```powershell
$PSVersionTable | Select-Object PSVersion, PSEdition
Get-Volume | Where-Object DriveLetter | Sort-Object DriveLetter | Format-Table DriveLetter, FileSystemLabel, FileSystem, SizeRemaining, Size -AutoSize
```

It takes about six hundred keys: 595 on a clean Windows 11. KeyJutsu knows a command has finished because the
shell says so, with a mark its prompt prints, not because some time has
passed. A slow `Get-ComputerInfo` just takes longer to finish.

### 3. Take the terminal back

After the last command, KeyJutsu still holds the keyboard, so the key you
were mid-way through pressing doesn't become the start of a command of its
own. Press the disarm chord:

**Ctrl+Alt+Shift+K**

The terminal is yours again, as an ordinary shell. Type `exit` and press
Enter to leave. On the way out, KeyJutsu says how each command went:

```text
 1. Describe this computer: succeeded (exit 0)
 2. Show the PowerShell version: succeeded (exit 0)
 3. List the volumes: succeeded (exit 0)
```

"Succeeded" comes from each command's real exit code, reported by the shell.
In `cmd`, which can't report one, it says "success unverified" instead of
guessing.

## The keys

| Key | Does |
| --- | --- |
| Any ordinary key | types the next character of the command |
| Any key at the end of a command | submits it |
| Ctrl+C | a real interrupt, always, to whatever is running |
| Esc | goes to a running command; ignored while KeyJutsu is typing |
| Ctrl+Shift+K | pause and resume (CLI); private operator controls (desktop) |
| Ctrl+Alt+Shift+K | disarm at once, erasing anything half-typed |

In the desktop app, Ctrl+Shift+K pauses the performance and opens the
operator controls, small and in a corner, for you rather than anyone watching:

![A performance, paused. The terminal shows Get-ComputerInfo -Property OsNa, typed so far by 31 keys. In the corner, the operator controls: step 01, Describe this computer, paused, 31 of 89 characters typed, next Show the PowerShell version, 0 of 3 done, with Resume and Disarm.](../images/performance.png)

The disarm chord works in every state, and it's handled before anything else
looks at the key, so nothing can hold on to it. Esc isn't the disarm, because people press it by reflex, and a performance
shouldn't end on a twitch.

## Try the other modes

```powershell
keyjutsu demo --clean --mode assisted
keyjutsu demo --clean --turbo
keyjutsu demo --clean --mode auto
keyjutsu demo --clean --submit enter
```

`assisted` types a short burst per key, up to the end of a word. `--turbo`
advances a whole word per key. `auto` types and runs everything by itself
while you watch; it has no authority the other modes lack. `--submit enter`
means only a real Enter submits a finished command, if you want a moment to
read it first. [Performing](../performing.md) has every mode and option.

## Your own commands

To perform commands you choose, rather than the demo's:

```powershell
keyjutsu perform --clean -c "Get-Service -Name Winmgmt" -c "Get-Date"
```

These are yours: there's no plan, no validation and no approval, and they run
exactly as if you'd typed them. Anything that changes the machine belongs in
a plan instead, which is the next guide.

Next: [getting a plan from an agent](planning.md).
