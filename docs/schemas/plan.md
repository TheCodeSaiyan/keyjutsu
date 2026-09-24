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

## What the schema cannot check

JSON Schema cannot see that an edge points to a step that does not exist,
that the graph has a cycle, or that `depends_on` and `edges` disagree. Those
are Milestone 3's job, in Rust, and will have fixtures of their own. The
schema is the first gate, not the only one.

## Fixtures

| Fixture | Refused because |
| --- | --- |
| `valid/docker-backend-branch.json` | (passes) a branching plan with facts, checks and effects |
| `invalid/agent-claims-readiness.json` | a proposal carries the KeyJutsu-owned section |
| `invalid/free-form-condition.json` | a condition is an expression string |
| `invalid/embedded-newline.json` | a command contains a carriage return |
| `invalid/staged-credential.json` | a credential step has a command to type |
| `invalid/future-major-version.json` | schema version 2.0 |
| `invalid/two-keys-in-one-condition.json` | a condition object has two keys |
