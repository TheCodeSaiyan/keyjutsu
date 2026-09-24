# The plan schema, version 1.0

`schemas/plan/v1/plan.schema.json` describes a stored plan.
`proposal.schema.json` describes what an agent may return. A plan is data,
never prose, and it is checked against these before KeyJutsu reads a word of
it.

`pnpm schemas:check` validates every fixture under `examples/`: the valid ones
must pass, and each invalid one must fail. If a schema change makes an invalid
fixture pass, the check fails, which is the point: the fixtures record what
the schema exists to refuse.

## Who writes what

The single most important line in the schema is the split between the agent's
part and KeyJutsu's part.

- **Anything outside `keyjutsu`** may come from an agent: steps, commands,
  shells, conditions, proposed risk, expected effects, validation and
  recovery.
- **Everything under `keyjutsu`** is KeyJutsu's own: readiness, proof level,
  remaining uncertainty, evidence, assessed risk, step hashes and approval.

The proposal schema is the plan schema plus one rule: no `keyjutsu`. An agent
cannot hand KeyJutsu a plan that says it is ready or approved, because such a
plan does not parse. Risk shows the same idea at field level: the agent's view
is `proposed_risk`, and KeyJutsu's view is `keyjutsu.steps.<id>.assessed_risk`.

## Shape

```text
plan
├── schema_version        "1.0"; unknown majors are refused, not guessed at
├── plan_id, task_id, title
├── target                { id: "local", kind: "local_windows" }
├── agent                 which agent, which version
├── environment_assumptions[]   each with a checkable condition
├── requirements[]        tools the plan needs
├── phases[]              for plans that cross a restart or sign-out
├── steps[]
├── edges[]               control flow, optionally conditional
├── execution_preferences
└── keyjutsu              KeyJutsu-owned state (never in a proposal)
```

A step has a `kind`: `command`, `validation`, `manual`, `user_input` or
`credential`. Command and validation steps must bind a shell and list
commands; a command is never shell-agnostic (§14). A credential step can have
no commands and is always `user_input`, so no secret can ever be staged typing.

Command text is a single line with no control characters. A newline in a
staged command would submit part of it, which is exactly the accidental
submission Performance Mode exists to prevent; the engine refuses the same
thing independently.

## Graphs and conditions

Without edges, steps run in order. Edges make a graph, and an edge with `when`
is followed only if its condition holds. That is how the fixture
`docker-backend-branch.json` goes one way for a WSL 2 backend and another for
Hyper-V, then joins again at verification.

Conditions are a closed vocabulary, one key per object:

| Condition | Holds when |
| --- | --- |
| `all`, `any`, `not` | the usual combinations |
| `step_outcome` | a step succeeded, failed or was skipped |
| `exit_code` | a step's exit code equals a value |
| `fact` | a fact KeyJutsu collected (say `docker.backend`) equals a value |
| `tool_version` | an installed tool satisfies a version range |
| `path_exists` | a path exists |
| `service_state` | a service is running, stopped, paused or missing |

There is deliberately no expression or script form. KeyJutsu evaluates
conditions, and it only evaluates these. Agents name facts; they never supply
their values.

Internal validation checks follow the same rule (`exit_code`,
`service_state`, `path_exists`, `file_sha256`, `json_value`, `tcp_port_open`,
`http_status`, and `timeout_seconds` for waiting).

## How a plan is read

`keyjutsu-plan` reads a plan through four gates, each of which can only
narrow what gets through (`crates/keyjutsu-plan/src/parse.rs`):

1. **Size and syntax.** Over 1 MiB, or not JSON, is refused. Nesting deeper
   than serde_json's limit of 128 is refused as bad JSON rather than
   overflowing the stack.
2. **Version.** `schema_version` is read first. Anything but `1.0` is refused
   by name ("schema version 2.0 is not supported; this KeyJutsu reads 1.0"),
   never interpreted by guessing (§50).
3. **Schema.** The document is checked against the schemas compiled into
   KeyJutsu. The one external reference they make, proposal to plan, is
   resolved from memory; anything else is refused, so no schema is ever
   fetched.
4. **Structure.** What JSON Schema cannot express, below.

Error messages never quote the document back. The validator's own messages
do, which for a problem at the top level means the entire plan: unreadable,
and a copy of task content in every error.

Try it on any file:

```powershell
keyjutsu plan check .\plan.json            # as an agent's proposal
keyjutsu plan check --stored .\plan.json   # as a stored plan
```

## What the structure check adds

| Problem | Why the schema cannot see it |
| --- | --- |
| A step id used twice | Uniqueness across array items that are objects |
| An edge, `depends_on`, phase or condition naming a step that does not exist | Cross-references |
| A cycle, reported with the steps on it: `detect-backend -> wsl-path -> verify -> detect-backend` | Graph shape |
| A condition asking about a step that cannot have run yet | Needs the graph's order |
| A step waiting on one in a later phase, a step in two phases, or in none | Needs the graph's order |
| A version range that does not parse, such as `>=7.*` | Grammar beyond a character class |
| KeyJutsu state recorded for a step that does not exist | Cross-references |

Every problem in a plan is reported at once, not just the first, so an agent
asked to revise a plan sees everything that is wrong in one go.

## Control flow

KeyJutsu, not the agent, decides what runs next (§10).
`keyjutsu_plan::frontier` takes the plan and what has happened so far and
returns the steps that are ready, the steps no branch leads to, and whether
the plan has halted or finished:

- A plan with no edges runs its steps in the order written.
- A step with no incoming edges is an entry. Any other step is taken when at
  least one incoming edge is: its source succeeded and its condition holds.
- A join waits until every path into it is decided, so a step after two
  parallel branches does not start while one is still running. The untaken
  side of a branch is skipped, which is what lets the two sides meet again.
- A step waits for everything in `depends_on`; if one of those was skipped,
  it is skipped too.
- Any failure halts the plan, whatever the graph says (§2.3).
- The order is deterministic: every step after its predecessors, ties broken
  by the order the plan lists them in.

## Conditions without guessing

Conditions are evaluated with three values. A fact KeyJutsu has not
collected is **unknown**, not false. `all` and `any` follow Kleene's rules, so
an unknown only matters when it could change the answer, and when it does,
the walk reports exactly which facts it needs instead of choosing a branch.
A property test checks that learning more facts can settle an unknown but
never flips an answer already given.

## Version ranges

Windows tools do not report semantic versions (Windows itself is
`10.0.26200.9457`; Git for Windows is `2.39.2.windows.1`), so a version is its
leading run of numbers, compared part by part.

| Range | Means |
| --- | --- |
| `7.4` | any 7.4.x (a bare version is a prefix) |
| `7.*` | any 7.x |
| `>=7.4 <8` | both must hold |
| `<5 \|\| >=7` | either may hold |
| `^28` | at least 28, below 29 |
| `^0.2.3` | at least 0.2.3, below 0.3 |
| `~1.2.3` | at least 1.2.3, below 1.3 |

## What a change affects

`keyjutsu_plan::diff` compares two versions of a plan and lists what was
added, removed and changed, field by field. A change to what a step *does*
makes it and every step after it in the graph need revalidation; a change to
its title, objective or reason does not. Changing the target, requirements or
environment assumptions affects every step. This is §11's "Step 3 changed;
steps 4, 5 and 7 need revalidation", and it is what approval (Milestone 4)
will use to decide which approvals a change withdraws.

## Fixtures

| Fixture | Outcome |
| --- | --- |
| `valid/docker-backend-branch.json` | passes: a branching plan that joins again |
| `valid/restart-boundary.json` | passes: two phases across a Windows restart |
| `invalid/agent-claims-readiness.json` | schema: a proposal carries the KeyJutsu-owned section |
| `invalid/free-form-condition.json` | schema: a condition is an expression string |
| `invalid/embedded-newline.json` | schema: a command contains a carriage return |
| `invalid/staged-credential.json` | schema: a credential step has a command to type |
| `invalid/future-major-version.json` | version: 2.0 |
| `invalid/two-keys-in-one-condition.json` | schema: a condition object has two keys |
| `structure-invalid/cycle.json` | structure: a cycle |
| `structure-invalid/edge-to-missing-step.json` | structure: an edge to `verfy` |
| `structure-invalid/condition-on-later-step.json` | structure: a branch asks about a step after it |
| `structure-invalid/phases-out-of-order.json` | structure: phase order contradicts the graph |
| `structure-invalid/bad-version-range.json` | structure: `>=7.*` |
| `structure-invalid/duplicate-step-id.json` | structure: `wsl-path` twice |

`pnpm schemas:check` requires the structure-invalid fixtures to *pass* the
schema, which is what shows the Rust checks are catching something the schema
cannot. `cargo test -p keyjutsu-plan` requires each to fail with its specific
problem.
