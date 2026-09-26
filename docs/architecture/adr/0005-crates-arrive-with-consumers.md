# 0005: Crates are created when something uses them

Status: accepted.

## Context

The planned layout had nine crates, and empty abstractions and artificial
micro-crates were to be avoided. Creating all nine at the start would have
meant six crates with no code, each an invitation to put something in the
wrong place.

## Decision

The first three crates were `keyjutsu-terminal`, `keyjutsu-execution` and
`keyjutsu-core`, because the first code needed them. Each later crate was
created by the first change that needed it. The [architecture overview](../overview.md) shows the crates that exist.

## Consequences

A reader of the tree sees only what exists. Two of the planned crates never
arrived: credential handling and the encrypted store turned out to belong in
`keyjutsu-core`, beside the code that uses them.
