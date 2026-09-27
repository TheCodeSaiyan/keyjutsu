# 0020: A conversation beside the plan, where what needs you can be answered

Status: accepted, 27 September 2026. Phase 1 is in 0.1.3; phase 2 followed.

## Context

The first real plans run in the app (print a cat photo to a PDF on the
desktop, then open it) showed that the plan workspace tells the operator
what is wrong but rarely lets them answer it where they read it:

- A finding says what is wrong ("cat.jpg is not pinned yet", "`magick` was not
  found", "the agent rated this Low; KeyJutsu rates it High"), and the fix is
  a button elsewhere, a command to type in another program, or guidance to
  write from nothing in a box that starts empty.
- A second agent's review adds concerns to a step. They can be read, not
  answered: nothing says "I've seen this, it's fine", or "ask the agent to
  deal with it".
- The agent often has to guess ("your Desktop is in OneDrive, so that is
  where it goes") where asking would be better. It cannot ask: a plan has no
  way to carry a question.
- How the plan runs (its mode, and a step's own) was chosen away from the
  plan.

Several of these were patched in 0.1.2: a step's own Stage downloads button,
Use KeyJutsu's rating, retry guidance written from the findings, and the run's
mode beside Arm. Each is a fix in one place; the pattern is the same each
time: something needs the operator, and the answer should be offered where it
is asked.

What must not change: the plan is the authority, the agent is never execution
authority, and nothing an agent writes is trusted without validation and the
operator's approval. A conversation must not become a way round any of that.

## Options

1. **Keep adding buttons to findings.** Cheap, and each one helps, but the
   step panel grows a button per kind of finding and still has nowhere for a
   review concern, an answer, or the agent's question.
2. **A chat with the agent.** Free-form, familiar. But a chat that edits the
   plan invites the agent to act on what it says rather than on what KeyJutsu
   validated, and an operator can't tell which of its messages changed
   anything.
3. **A conversation of things to answer, beside the plan and each step.**
   Everything that needs the operator arrives as an item with choices, and
   free text is always possible. Each choice is one of the workspace's own
   operations; the conversation holds no authority of its own.

## Decision

Option 3, in two phases.

**Phase 1: KeyJutsu's findings and review concerns, answerable.** No change
to the plan format.

- The plan and each step have a conversation, shown with the plan's notes and
  in the step panel. It holds the agent's notes as now, and an item for each
  thing that needs the operator.
- **KeyJutsu writes the choices for its own findings,** from a fixed table,
  never from agent text. For example:

  | Finding | Choices |
  | --- | --- |
  | a download not pinned | Stage it · Remove the step |
  | a command or tool not found | Ask for a step without it · I've installed it, validate again · Get it (a known download link) |
  | a precondition that does not hold | Ask the agent to adjust the step · I've changed the machine, validate again · Remove the step |
  | a mistyped command or parameter, broken syntax | Ask the agent to fix it · Edit the step |
  | a failed dry run | Ask the agent to fix it · Edit the step |
  | the agent rated it lower, at high or critical | Use KeyJutsu's rating · Ask the agent why |

- **A review concern can be answered:** Ask the agent to address it (a retry
  of the step, with the concern as guidance), It's fine (dismissed, with the
  operator's reason recorded in the plan's provenance), or Edit the step.
- **Free text is always there.** In a step's conversation it goes to the
  agent as guidance for that step (the retry that exists now); in the plan's,
  as guidance for the whole plan (the revision that exists now); or it is kept
  as the operator's own note.
- Every choice is an operation the workspace already has: stage, accept a
  rating, retry or revise with guidance, edit, remove, validate, note. Anything
  the agent sends back is a draft change that returns unvalidated and
  unapproved, as now.
- The run's mode (Runs in) and a step's own (How this step runs) are chosen on
  the plan, as in 0.1.2 onwards.

**Phase 2: the agent asks.**

- The plan format gains optional `questions`: each with an id, the step it is
  about (or none, for the plan), the question, a few options, and whether free
  text is allowed. A proposal may carry them; KeyJutsu-owned fields stay
  refused as now.
- The agent is told it may ask instead of guessing. Questions appear in the
  conversation; answering sends the question and answer back as guidance, and
  the agent's revision comes back as any other: unvalidated, unapproved.
- An unanswered question does not block approval by itself; what the plan does
  is judged by validation, as always. The operator can answer "Carry on as
  planned".

As built, a question's options are the agent's words, but what choosing one
does is fixed: the answer goes to the agent as guidance, redacted like all
guidance. Questions and their options are held to the same rule as commands,
so an invisible or reordering character is refused. The question is closed
once answered, even if the agent's revision asks it again, and no step hash
covers questions. The CLI has `keyjutsu plan answer`.

## Consequences

- The step panel becomes: what the step does, its findings as conversation
  items with their choices, then its details.
- Dismissing a concern, and every answer, is recorded in the plan's provenance
  with who and when, so Save plan and the history show what was decided.
- Phase 2 changes the plan schema (a new optional field, the same major
  version), the agent prompts, and the recorded answers the agent tests
  replay.
- Tests: each finding kind produces the choices in the table, and each choice
  calls the operation it names; free text in a step goes to that step's retry
  and nowhere else; a dismissed concern is in the provenance; an agent's
  question round-trips (phase 2).
