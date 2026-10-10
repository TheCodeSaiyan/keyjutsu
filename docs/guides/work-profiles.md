# Work profiles

A work profile says what kind of work a plan is for, and KeyJutsu asks more
of the plan to match. Under IT Operations every change has to say how to
recover from it; under Helpdesk nothing may touch a disk; under Audit only
nothing may change the machine at all. The agent is told the profile's rules
before it writes a word, and validation enforces them, so a plan that breaks
one is not ready, however it was written.

There are six built in, and you can write your own on top of them. Which
one applies is set for the machine, or for a folder and everything in it, and
you can always choose another by hand.

In the desktop app, the profile is under the task box: its name, why it
applies, and **Change…**. Plan stays unavailable until one applies. When a
folder asks for less, can't be used, or nothing is set, the same place says
so and offers the choice. A profile picked there with **Use for this window**
lasts until the window closes. This page shows the CLI, which does the same.

## The six profiles

```powershell
keyjutsu profile list
```

```text
developer          Developer
packaging          Packaging, extends developer
helpdesk           Helpdesk, extends developer
it-operations      IT Operations, extends helpdesk
security-response  Security response, extends it-operations
audit-only         Audit only, extends security-response
```

Each profile extends one other and can only tighten it. IT Operations is
everything Helpdesk asks and more; Audit only is everything Security response
asks and more. A profile that tried to loosen what it extends would be
refused when it was read, not quietly corrected, so a profile always means at
least what its parent means.

| Profile | For | What it adds |
| --- | --- | --- |
| Developer | your own machine and code | a recovery for critical steps; `git push` counts as high risk |
| Packaging | building and installing software | a recovery from high risk; installers and package managers count as high risk |
| Helpdesk | fixing someone's machine | a recovery from high risk; no code KeyJutsu cannot see into; local account changes count as high risk; disks, partitions and boot configuration refused |
| IT Operations | servers, directories, fleets | a recovery for every change (normal risk and above); each high-risk step approved on its own, with its typed phrase; Active Directory, Entra and Intune, Group Policy, DNS and DHCP changes count as high risk, removals as critical |
| Security response | an incident | log clearing and secure deletion refused; killing processes and deleting files count as high risk, since both destroy evidence |
| Audit only | looking, not touching | nothing above low risk: a step that changes anything is refused |

`keyjutsu profile show NAME` prints every rule a profile enforces, which
profile each one came from, and the exact text the agents are given.

## Which profile applies

1. **A folder's choice.** KeyJutsu looks for `.keyjutsu/profile.toml` in the
   folder a command runs in, then in each folder above it, and the nearest
   one wins. For `plan propose --folder DIR`, it looks from `DIR`.
2. **This machine's default**, if no folder chooses.
3. **Nothing set.** Making a plan stops and asks you to choose. Checking a
   plan file that already exists goes ahead without a profile, as it did
   before profiles existed, and says that none was applied.

Set the machine's default once:

```powershell
keyjutsu profile default it-operations
```

A folder chooses with one line:

```toml
# .keyjutsu/profile.toml
default = "security-response"
```

`keyjutsu profile which` says which profile applies where you are and why.
Every plan command says it too, on one line, before it starts.

### When a folder asks for less

A folder can't lower strictness without you noticing. If its profile isn't
the machine's default or one that extends it (Developer in a cloned
repository on an IT Operations machine, say), planning stops until you
confirm it:

```powershell
keyjutsu profile confirm
```

The confirmation covers the folder's `.keyjutsu` files exactly as they are.
Change them and you're asked again. The one exception is a Git checkout
turning LF into CRLF, which doesn't count as a change. Confirmations are kept
in KeyJutsu's encrypted store, against the folder's real path: reach the same
folder through a mapped drive, its UNC path or a junction, and the one
confirmation holds. A different folder with the same files asks again.

### When a folder can't be used

A folder whose `.keyjutsu` can't be trusted stops planning in that folder
until you choose a profile by hand. Any of these breaks it:

- a file KeyJutsu doesn't read, such as `notes.txt`. Only `profile.toml` and
  `profiles/*.toml` belong there;
- a profile that doesn't parse, doesn't resolve, or reuses a name already
  defined;
- a link or junction inside `.keyjutsu`, or a `.keyjutsu` that is one;
- a starting folder whose real path can't be found: an offline share, a
  disconnected drive, a folder you can't open.

Files an operating system, Git or an editor leaves behind (`desktop.ini`,
`Thumbs.db`, `.DS_Store`, `.gitkeep`, editor swap and backup files) are
ignored wherever they are, and kept out of the confirmation.
`keyjutsu profile which` lists any it found.

### Choosing by hand

`--profile NAME` on any plan command uses that profile instead, whatever the
folder or machine says, and the command says what it overrode. It lasts for
that one command. In the desktop app a pick lasts for the window, and is
dropped as soon as what it overrode changes: the folder's files, or the
machine's default.

## Profiles of your own

A profile of your own extends exactly one other and can only tighten it. Put
it in `%LOCALAPPDATA%\KeyJutsu\profiles\` to use it anywhere on this
machine, or in a folder's `.keyjutsu\profiles\` for that folder alone:

```toml
# %LOCALAPPDATA%\KeyJutsu\profiles\client-x.toml
name = "client-x"
title = "Client X"
extends = "it-operations"
instructions = "Mail is the client's own team's: never change a mailbox."

[[validation.deny]]
commands = ["Set-Mailbox", "Remove-Mailbox"]
reason = "changes a mailbox, which the client's team owns"
```

A folder's profile can extend one from the machine (`extends = "client-x"`).
Names must be unique across the built-ins, the machine and the folder: a name
defined twice breaks the folder, so nobody ends up under a profile that
differs from the one they read.

What a profile can set:

| Key | What it does |
| --- | --- |
| `instructions` | text added to what the agents are told |
| `validation.require_recovery_from` | `low`, `normal`, `high` or `critical`: steps at that risk or above need a recovery |
| `validation.max_risk` | the highest risk a step may have |
| `validation.block_opaque_code` | `true` refuses code KeyJutsu cannot see into |
| `[[validation.raise]]` | `commands`, `to` and `reason`: those commands count as at least that risk |
| `[[validation.deny]]` | `commands` and `reason`: those commands are refused |
| `approval.confirm_each_from` | steps at that risk or above need their own typed phrase to be approved, as critical steps always do |

Any other key is refused, so a mistyped one is caught rather than ignored.
`approval.second_person` is reserved for two-person approval, which KeyJutsu
doesn't have yet: setting it is refused, so nobody believes a second person
is being asked when nobody is.

## Planning under a profile

Ask for the plan as usual; the profile that applies is named first:

```powershell
keyjutsu plan propose "Disable the departed user's account" --agent claude --out disable.json --send
```

The profile's rules go to the agent after KeyJutsu's own ground rules, and to
a reviewer too, with `keyjutsu plan review`. An agent that knows a change
needs a recovery usually writes one first time, rather than being sent back
for it.

## Validating under a profile

Validation and approval apply the same profile:

```powershell
keyjutsu plan validate disable.json
keyjutsu plan approve disable.json --out disable.snapshot.json
```

The profile's findings appear under the check `work profile`, beside
everything else validation found. The example plan that clears a build cache
deletes a folder outright, with no way back, and Helpdesk wants a way back
for anything high risk or above:

```powershell
keyjutsu plan validate docs/examples/clear-build-cache.json --profile helpdesk
```

```text
  INVALID    Medium  risk Critical clear-cache
      ✕ work profile: Helpdesk needs a recovery for every step at high risk or above, and this critical step has none
      ? Checked without your PowerShell profile: aliases and functions it defines were not considered
      ? What this step changes is only proven by running it

1 of 1 step is not ready.
```

What each kind of finding does to a step:

- **A refused command** makes the step BLOCKED, and names the rule and the
  profile it came from.
- **A missing recovery** makes the step INVALID: the plan as written does
  not meet the profile, and the agent can fix it.
- **A step above the profile's risk ceiling** is BLOCKED.
- **Code KeyJutsu cannot see into**, under a profile that refuses it, is
  BLOCKED. That is a command whose name is held in a variable, a script file
  handed to a shell, `Invoke-Expression` fed by a pipeline, or nested code
  whose arguments are computed when it runs.
- **A raise rule** lifts the step's risk and says why in its risk reasons,
  which can in turn bring in the recovery rule.

A step that meets the profile records that it did, so the report shows the
profile was applied rather than leaving you to assume it.

## Approving under a profile

The plan records the profile it was validated under: its name, the chain it
extends, where each came from, and why it applied. That record is sealed into
the snapshot with everything else, so a snapshot says what it was approved
under, and altering the record afterwards breaks the seal.

Approving the plan normally covers every step except critical ones, which
each need their own typed phrase. A profile can ask for the same from a lower
risk. IT Operations does it from high risk:

```text
Not sealed: these steps need their own typed confirmation.

  CONFIRM          stop-spooler: Stop the spooler (IT Operations asks for its own confirmation from high risk)
```

In the desktop app the step's confirmation opens as it does for a critical
step, headed "Confirm this step" rather than "Critical action", and it shows
what the step runs, what it acts on and how it would be undone before the
phrase. The same step is confirmed again just before it runs if its approval
is over an hour old, exactly as a critical step is.

Validating under a stricter profile after approving doesn't get round it:
sealing checks every step against the profile recorded in the plan as it is
sealed, so a step approved without its phrase under the old profile has to be
confirmed under the new one.

## What a deny rule matches

Rules match commands as PowerShell parses them, never by searching the text
of the line. An alias is matched by what it stands for, so a rule against
`Get-ChildItem` also catches `gci`. A module-qualified name, a full path and
an extension are ignored, so `format`, `FORMAT.COM` and
`C:\Windows\System32\format.com` are one command.

Wrapping a command in another shell doesn't get it past. KeyJutsu unpacks,
without running anything:

- `pwsh -Command` and `powershell -Command`, and the bare command form of
  Windows PowerShell;
- `-EncodedCommand`, decoded from its Base64;
- `cmd /c` and `cmd /k`, split into the commands they run;
- `Invoke-Expression` given one literal string;
- `Start-Process`, by the program it starts and its arguments.

What it unpacks is parsed and matched like any other line, up to four levels
deep. So `cmd /c pwsh -c "Format-Volume -DriveLetter Z"` is refused under
Helpdesk exactly as `Format-Volume -DriveLetter Z` is.

A rule can name the start of the arguments too: `wevtutil cl*` refuses
clearing an event log but not querying one.

## The change record

Every run kept in the history has a change record: the record to attach to a
ticket for the change you made.

```powershell
keyjutsu history list
keyjutsu history change-record 20261010-120000-cafe --out change.md
```

It says, in Markdown, or JSON with `--json`:

- the task, the agent that planned it, when it ran, and the Windows account
  that ran it;
- how it ended;
- the work profile and why it applied, every profile in its chain with where
  it came from and a hash of its text, the folder the shell started in and
  where that really is, and any leftovers ignored in `.keyjutsu`;
- each step: its risk, when it was approved and whether with its own typed
  confirmation and why, how it would be undone, its result, exit code and
  checks, and what it ran;
- what each step printed, if the run was recorded. Otherwise only a failed
  step's output, which is kept with the outcome either way. Secrets are
  redacted from output as in every export; names are not, because saying who
  did what is the point of the record;
- the Git changes the run made.

It's built from the encrypted history each time you ask, so nothing is
written in plain text until you choose where.

## Limits

- **Nested code costs a second look.** Each level of nested PowerShell is
  parsed by starting PowerShell once more, only when a plan has any.
- **What KeyJutsu cannot unpack, it cannot match.** A `[ScriptBlock]::Create`
  call, a .NET method that starts a process, or a script file's contents are
  not looked into. Helpdesk and the profiles that extend it refuse the forms
  KeyJutsu can recognise as opaque; Developer and Packaging allow them, and
  their deny lists are correspondingly weaker.
- **A step the operator carries out** (manual, user input, credential) needs
  no recovery under any profile: KeyJutsu cannot run one for it.
- **cmd.exe lines** are split on `&` and `|` outside quotes, not parsed by
  cmd itself, as everywhere else in validation.
- **The change record comes from the CLI.** The desktop app keeps the same
  history, so `keyjutsu history change-record` works for its runs too.
- **Runs from before this release** have no account in their record, and
  their change record says so.
- **One person approves.** Two-person approval is reserved in the format and
  refused until it exists.
