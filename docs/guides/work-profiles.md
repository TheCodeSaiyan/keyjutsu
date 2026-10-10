# Work profiles

A work profile says what kind of work a plan is for, and KeyJutsu asks more
of the plan to match. Under IT Operations every change has to say how to
recover from it; under Helpdesk nothing may touch a disk; under Audit only
nothing may change the machine at all. The agent is told the profile's rules
before it writes a word, and validation enforces them, so a plan that breaks
one is not ready, however it was written.

There are six, built into KeyJutsu. Profiles of your own, and choosing one
per folder or per machine, come in a later release; for now you name the
profile on the command line.

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
| IT Operations | servers, directories, fleets | a recovery for every change (normal risk and above); Active Directory, Entra and Intune, Group Policy, DNS and DHCP changes count as high risk, removals as critical |
| Security response | an incident | log clearing and secure deletion refused; killing processes and deleting files count as high risk, since both destroy evidence |
| Audit only | looking, not touching | nothing above low risk: a step that changes anything is refused |

`keyjutsu profile show NAME` prints every rule a profile enforces, which
profile each one came from, and the exact text the agents are given.

## Planning under a profile

Add `--profile` when you ask for the plan:

```powershell
keyjutsu plan propose "Disable the departed user's account" --agent claude --profile it-operations --out disable.json --send
```

The profile's rules go to the agent after KeyJutsu's own ground rules, and to
a reviewer too, with `keyjutsu plan review --profile`. An agent that knows a
change needs a recovery usually writes one first time, rather than being sent
back for it.

## Validating under a profile

Validate and approve with the same profile:

```powershell
keyjutsu plan validate disable.json --profile it-operations
keyjutsu plan approve disable.json --profile it-operations --out disable.snapshot.json
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
- **The profile is not yet part of the approval.** A snapshot approved under
  IT Operations does not record that it was; that comes with the change
  record.
