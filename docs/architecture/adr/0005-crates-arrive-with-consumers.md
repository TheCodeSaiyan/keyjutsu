# 0005: Crates are created when a milestone gives them a consumer

Status: accepted, Milestone 0.

## Context

The specification suggests nine crates and also says not to build empty
abstractions or artificial micro-crates (§4, §59). Creating all nine now would
mean six crates with no code, each an invitation to put something in the wrong
place.

## Decision

Milestones 0 to 2 create three crates, `keyjutsu-terminal`,
`keyjutsu-execution` and `keyjutsu-core`, because those milestones have code
for them. Each remaining crate is created by the milestone that first needs
it; [the dependency graph](../dependency-graph.md#crate-order) lists which.

## Consequences

The layout in the specification is the target and the overview shows it. A
reader of the tree today sees only what exists.
