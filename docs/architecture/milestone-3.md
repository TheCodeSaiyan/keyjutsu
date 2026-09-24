# Milestone 3: the plan model

What was built, and what each "done when" in §60 rests on. A criterion marked
met has a named test or a command you can run behind it.

## What it is

A new crate, `keyjutsu-plan`, with no I/O:

- **`model`**: the plan as Rust types, field for field with
  `schemas/plan/v1/plan.schema.json`.
- **`parse`**: four gates (size and syntax, version, schema, structure),
  with the schemas compiled in and nothing ever fetched.
- **`graph`**: control flow from edges (or the order written, when there are
  none) plus `depends_on`; a deterministic execution order; every structural
  problem, collected rather than stopping at the first.
- **`condition`**: three-valued evaluation of the closed condition vocabulary,
  reporting which facts it still needs rather than guessing.
- **`walk`**: which steps are ready, which are skipped, whether the plan has
  halted or finished.
- **`diff`**: what changed between two plans and which steps that puts in
  question.
- **`version`**: version ranges that cope with what Windows tools report.

The front ends reach it through `keyjutsu-core` (`keyjutsu_core::plan`), as
they do everything else. The CLI has `keyjutsu plan check <file>`.

## Done when

| §60 criterion | Status | Evidence |
| --- | --- | --- |
| Plans parse deterministically | Met | `parsing_is_deterministic_and_round_trips`, `key_order_in_the_input_does_not_matter`; property tests `mutated_plans_never_panic_and_always_get_the_same_answer` and `whatever_parses_serialises_to_something_that_parses_the_same`, 2,000 cases each. |
| Invalid plans are rejected | Met | Every `invalid/` fixture is refused at the schema or version gate, every `structure-invalid/` fixture with its specific problem (`structure_invalid_fixtures_report_the_right_problem`); `arbitrary_text_is_refused_cleanly`. |
| Graph cycles are detected | Met | `cycle.json` reports the cycle's steps in order. |
| Conditions use constrained typed semantics | Met | Closed vocabulary in schema and model; `kleene_logic_decides_when_it_can`, `a_missing_fact_is_unknown_and_named`, property tests `double_negation_changes_nothing` and `a_decided_condition_stays_decided_when_more_is_known`. |
| TypeScript types generated and aligned | Met | 57 plan types in `packages/types/src/generated/plan`, exposed as the `plan` namespace; CI fails if they drift. |
| Plan modifications are trackable | Met | `diff` with dependency-aware `affected`; §11's own example is the test `changing_a_step_affects_it_and_everything_after_it`. |

## Checked by breaking it

Two deliberate faults, each caught:

- letting a join start as soon as one incoming path was taken failed two
  control-flow tests;
- removing the check on conditions that ask about later steps failed the
  fixture test.

## Found on the way

- **Generated types were overwriting each other.** ts-rs names each file after
  the type, and the plan crate's `Check` replaced the readiness `Check`; its
  `ExecutionMode` replaced the engine's, unnoticed only because the two
  happen to match. Which one survived depended on test order. Crates now
  export to folders of their own; see [ADR 0007](adr/0007-typescript-generated-from-rust.md).
- **Schema errors quoted the whole plan.** The validator's messages include
  the offending value, which for a top-level failure is the entire document.
  Messages now name the place and the rule; a test keeps them that way.

## Not done, and why

- **Coverage-guided fuzzing.** §55 asks for fuzzing of the parser and the
  condition evaluator. What exists is property testing: 8 properties, 2,000
  generated cases each, about 1.3 seconds. Real fuzzing needs cargo-fuzz on
  nightly Rust and belongs with Milestone 17's security hardening. Recorded as
  [deviation D12](deviations.md#d12-property-tests-stand-in-for-fuzzing-until-milestone-17).
- **Nothing runs plans yet.** The walker decides what would run next; running
  it is Milestone 8. Facts come from Milestone 5's validation.

## Next

Milestone 4: approval. The hashing proposed in
[ADR 0010](adr/0010-plan-hashing.md) and the `diff` built here are the two
halves of "any execution-relevant change after approval invalidates it".
