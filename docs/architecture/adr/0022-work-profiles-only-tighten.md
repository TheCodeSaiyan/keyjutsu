# 0022: Work profiles only tighten, and match commands as PowerShell parses them

Status: accepted.

## Context

Operators wanted KeyJutsu to be stricter for some kinds of work than others:
a change to Active Directory deserves more than a change to a scratch folder
on a developer's laptop. That strictness has to be something an operator can
rely on. A profile that a later file could quietly weaken, or a deny list
that a wrapper such as `cmd /c` slipped past, would be worse than none,
because it would be trusted.

## Decision

**Profiles are TOML, and six are built in.** Developer is the root; every
other profile extends exactly one other. A profile can add instructions for
the agents, raise the risk of named commands, require a recovery from a given
risk upwards, cap the risk a step may have, refuse code KeyJutsu cannot see
into, and refuse named commands.

**Each link can only tighten.** Resolving a profile walks its chain from the
root down. A recovery threshold can only move down, a risk ceiling can only
move down, and refusing opaque code can only be switched on. A profile that
would loosen any of these is an error naming both profiles, never clamped
silently: clamping would leave a file that says one thing and means
another. Raise and deny rules only accumulate; there is no syntax to remove
one. Unknown keys are refused, cycles are refused, and chains are limited to
twelve profiles.

**Rules match parsed commands, never raw text.** The analysis shell already
parses every line with PowerShell's own parser (ADR 0014). It now also
reports what an alias stands for, so a rule matches `gci` as `Get-ChildItem`.
Names are compared without module qualifiers, folders or program
extensions.

**Nested code is unpacked statically.** `pwsh -Command`, `-EncodedCommand`,
`cmd /c`, `Invoke-Expression` with a literal string and `Start-Process` are
turned into the code they would run, which is parsed in a further round of
analysis and matched like any other line, up to four levels deep. Nothing a
plan names is run to do this; ADR 0014 holds unchanged. What cannot be
unpacked without running something is reported as opaque, and the profiles
from Helpdesk up fail the step for it.

**Breaches are evidence like any other.** They are recorded under the check
`work profile`: a refused command or a risk above the ceiling is BLOCKED, a
missing recovery INVALID, since the agent can fix the plan.

## Consequences

- A command-line parser of PowerShell's own options lives in KeyJutsu. It
  recognises a fixed set of options and treats any other as opaque, so a new
  option in a future PowerShell fails closed rather than open.
- Each level of nesting costs one more start of the analysis shell, only for
  plans that nest and only under a profile.
- `[ScriptBlock]::Create`, .NET process APIs and script files' contents are
  not unpacked. The profiles that refuse opaque code catch the forms KeyJutsu
  recognises; the others cannot promise their deny lists hold against code
  written to evade them.
- Which profile applies comes from the nearest `.keyjutsu/profile.toml`
  above the starting folder's real path, then the machine's default. A
  folder default less strict than the machine's, judged by ancestry, waits
  for a confirmation kept in the encrypted store per real path and hash of
  the loaded files (CRLF read as LF). Anything unexpected in `.keyjutsu`, a
  link or junction in or as it, or an unresolvable starting folder stops
  planning until a profile is picked by hand. The machine's files are
  trusted: changing one never asks about a folder again.
- Recording the profile with an approval is later work.
