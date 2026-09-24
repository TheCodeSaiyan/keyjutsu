# Milestone 5: validation

What was built, and what each "done when" in §60 rests on.

## What it is

A new crate, `keyjutsu-validation`, and `keyjutsu plan validate`:

```text
$ keyjutsu plan validate plan.json
  READY      High    risk Low      detect-backend
      ? `wsl` is an external program; its arguments are not checked
  READY      Medium  risk Normal   wsl-path
      ? What this step changes is only proven by running it
  ...
All 4 steps are ready.
```

For each step it gathers the strongest safe evidence (§13):

| Check | How |
| --- | --- |
| Shell exists, version in range | locate the shell; its version from the analysis shell or `ver` |
| Syntax | PowerShell's parser, in the step's own edition |
| Commands resolve | `Get-Command`, without the user's profile |
| Parameters exist and are unambiguous | `CommandInfo.ResolveParameter` |
| Tools installed, version in range | resolved on `PATH`; version from the file's version resource |
| Working directory exists | the file system |
| Preconditions and environment assumptions | the Milestone 3 evaluator, with real service states, paths and tool versions |
| Privilege | Administrator steps are blocked until the broker (Milestone 10) unless KeyJutsu is already elevated |
| Risk | KeyJutsu's own rules, explained, raised but never lowered by the agent's label |
| Dry run | `-WhatIf`, only where it is trustworthy ([ADR 0014](adr/0014-validation-runs-nothing-it-validates.md)) |

Each step ends with a readiness (READY, NEEDS_REVIEW, BLOCKED, INVALID), a
proof level (NONE, LOW, MEDIUM, HIGH), the uncertainty that remains, and the
evidence behind each.

`keyjutsu plan approve` now validates first and will not seal unless every
step is READY; the report is sealed with the plan, so KeyJutsu's own risk
assessment decides which steps need a typed confirmation, including after the
snapshot is reloaded. That closes deviation D13.

## Done when

| §60 criterion | Status | Evidence |
| --- | --- | --- |
| Every executable step produces readiness, proof level, remaining uncertainty and evidence | Met | `judge` fills all four for every step; `the_report_can_be_recorded_in_a_stored_plan`. |
| No unvalidated mandatory step may be armed | Met for sealing; arming is Milestone 8 | `seal` refuses a validated plan with any step not READY (`a_validated_plan_only_seals_when_every_step_is_ready`); the CLI's approve always validates first (`a_plan_that_cannot_run_here_is_not_sealed`). |
| PowerShell AST | Met | `a_mistyped_parameter_or_broken_syntax_is_invalid`, in PowerShell 7 and 5.1. |
| Executable resolution, tool versions | Met | `a_missing_command_or_tool_blocks_the_step` (cmd.exe's version read from its file). |
| Shell and version parity | Met | commands are analysed in the step's own shell edition. |
| Path checks, permissions, preconditions | Met | `a_missing_working_directory_or_administrator_right_blocks_the_step`, `preconditions_and_assumptions_are_checked_against_the_machine`. |
| Native dry run | Met, narrowly | `validating_a_destructive_step_leaves_the_machine_alone`, `a_trailing_comment_cannot_turn_a_dry_run_into_a_real_one`, `expressions_are_never_evaluated_by_a_dry_run`. |
| File-copy staging | **Not built** | deviation D15. |

## Checked by breaking it

- Switching the dry run from `$WhatIfPreference` to appending ` -WhatIf`
  made validation **really delete** a file behind a trailing comment; the test
  caught it.

## Found on the way

- A tool named only inside a condition (`tool_version: cmd.exe` in an
  environment assumption) was never looked up, so the assumption came back
  undecided instead of false. Tools are now collected from conditions too.
- The text rule `"format "` would have rated `Get-Date -Format o` critical.
  Programs such as `format` and `shutdown` are now matched only as the command
  word.
- Wiring validation into approval showed that two CLI tests relied on
  fixtures needing Administrator or Docker, which validation now rightly
  blocks. The tests now build their plans from what every Windows machine has.

## Limits

- **No file-copy staging** (D15). §13 asks for transforming a disposable copy
  of a configuration file and checking the result. The plan schema has no way
  to express a structured transform yet; that needs a schema addition.
- **Named facts are never collected** (D16). A condition on a fact such as
  `docker.backend` is undecided until fact collectors exist, so such steps
  go to review.
- **Checked without the profile** (ADR 0014): profile-defined aliases and
  functions are reported as not found.
- **Dry runs cover one module.** Everything else is parsed and resolved but
  its effect is only proven by running it, and says so.
