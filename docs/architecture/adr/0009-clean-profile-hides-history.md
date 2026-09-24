# 0009: The clean profile turns off history predictions and saving

Status: accepted, Milestone 1.

## Context

Testing turned up two things about PSReadLine, which loads even with
`-NoProfile`:

- **Predictions draw on the user's history file.** While a staged command
  was being typed, PSReadLine showed grey suggestion text taken from history.
  In one spike that was a command from an unrelated repository on the same
  machine. During a performance, whatever the user once typed can appear on
  screen, including anything sensitive that ended up in their history.
- **Commands KeyJutsu types are saved to that history.** Before this
  decision, every real-shell test run added its commands to the developer's
  own PSReadLine history.

## Decision

With the clean profile, the integration script sets
`-HistorySaveStyle SaveNothing` and `-PredictionSource None` (the latter in a
`try`, because the PSReadLine 2.0 that ships with Windows PowerShell 5.1
has no predictions). All automated tests use the clean profile.

The detected profile is left exactly as the user configured it. That is the
point of the detected profile: the terminal should behave as their own does.

## Consequences

The threat model records predictions in the detected profile as a known
exposure. The compatibility profile (§16), when built, should offer
predictions off while keeping the user's appearance.
