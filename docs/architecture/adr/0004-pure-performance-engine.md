# 0004: The performance engine is a pure state machine

Status: accepted.

## Context

Performance Mode's guarantees are the product's credibility: only staged text
reaches the shell whatever key is pressed, an incomplete command is never
submitted, and the disarm chord always works. Guarantees like those are only
worth stating if they are tested exhaustively, and a component that owns
threads, timers and a pseudo-console is awkward to test exhaustively.

## Decision

`PerformanceEngine::handle(input) -> Vec<Action>`. Inputs are keys (already
classified), prompt marks, operator actions and pacing ticks; actions are
bytes to write, ticks to schedule, state changes and events. The engine does
no I/O, owns no threads and reads no clock.

`keyjutsu-core` does the I/O: it carries out actions in order while holding
the session lock, runs pacing ticks on a timer with a generation counter so a
cancelled tick cannot fire late, and feeds prompt marks back in.

The only place a staged Enter is produced is `submit()`, which is reached only
once every character has been delivered. Each state change is checked against
the table in `state.rs`.

## Consequences

The engine suite (21 tests, under a millisecond) covers the case the
engine exists for (mashed keys deliver exactly the staged command), every key class in every state that matters, and asserts
that every transition recorded was legal. Two deliberate mutations were run to
confirm the tests can fail: letting Enter submit mid-command broke two tests;
emitting the release before the final snapshot broke the real-shell disarm
test.

The engine cannot notice anything the core does not tell it. If the core
failed to deliver a mark, the engine would wait forever in `EXECUTING`; that
is visible (the operator can disarm) rather than silent, which is the
right way round.
