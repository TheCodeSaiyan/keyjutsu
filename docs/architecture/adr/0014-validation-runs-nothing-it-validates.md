# 0014: Validation runs nothing a plan names, with one guarded exception

Status: accepted.

## Context

Validation happens before approval. If it ran what a plan names, an agent's
plan would be executing on the operator's machine before the operator had
said yes, which is exactly what KeyJutsu exists to prevent. But validation
should still find the strongest safe proof, including native dry runs such as
`-WhatIf`, and knowing a tool's version needs *something* to be asked.

## Decision

**What validation does:**

- Parses every command line (steps, visible validation, recovery) with
  PowerShell's own parser, `Parser.ParseInput`, in a throwaway shell started
  with `-NoProfile`, in the step's own edition (PowerShell 7 or Windows
  PowerShell 5.1). Parsing builds a syntax tree; it runs nothing.
- Looks each command up with `Get-Command` and checks each parameter with
  `CommandInfo.ResolveParameter`, which also catches ambiguous abbreviations
  (`-P` on `Get-ChildItem`) and mistypes (`-Nmae`).
- Reads applications' versions from their files' version resources
  (`docker.exe` reports 29.5.2 this way), never by running `tool --version`.
- Reads service states with `Get-Service`, and path existence directly.

**The one exception:** a `-WhatIf` dry run, and only when every one of these
holds (`powershell::what_if_blocker`):

- the line is a single command, not a pipeline or several statements;
- that command is a compiled cmdlet from `Microsoft.PowerShell.Management`,
  whose cmdlets route every change through ShouldProcess;
- it declares `-WhatIf`, and the line does not set `-WhatIf` or `-Confirm`
  itself;
- its arguments are all literals, so nothing is evaluated by the dry run.

The dry run sets `$WhatIfPreference = $true` for the whole throwaway session
rather than appending ` -WhatIf` to the text.

## Why the preference, not an appended switch

Appending does nothing when the line ends in a comment:
`Remove-Item x # tidy up -WhatIf` deletes `x`. This was not hypothetical: with
the dry run changed to append, the test
`a_trailing_comment_cannot_turn_a_dry_run_into_a_real_one` failed with "the
dry run really deleted the file". With the preference it passes.

## Consequences

- Validation checks commands **without the user's profile**, so an alias or
  function the profile defines is reported as not found. That errs towards
  BLOCKED rather than towards trusting something KeyJutsu could not see, and
  every PowerShell step carries the uncertainty "Checked without your
  PowerShell profile".
- `Get-Command` may import a module to answer, which runs that module's own
  loading code. Modules come from the machine's module path, not from the
  plan, but it is recorded in the threat model.
- Most external programs cannot be dry-run and their arguments cannot be
  checked; steps using them say so in their uncertainty and cap at MEDIUM
  proof unless KeyJutsu's rules rate them read-only.
- A dry run that fails is not a block: an earlier step may create what a
  later step removes. It puts the step in review instead.
