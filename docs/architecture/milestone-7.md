# Milestone 7: the plan workspace

What was built, and what the "done when" in §60 rests on. It follows the
design kit's mockups 01 to 03 (new task, plan workspace, critical action).

## What it is

- **`keyjutsu-core::workspace`**: the draft plan, and every change to it.
  Each edit is re-read as a whole plan (schema, structure, graph) before it is
  kept, so an edit the plan would refuse never becomes the draft. A change
  keeps KeyJutsu's validation only where it cannot reach: the changed step and
  everything after it in the graph go back to "revalidation required" (§11).
  Renaming a step changes nothing that runs, so it keeps its validation.
- **Desktop screens:** a left rail (New task, Plan, Terminal); the new-task
  screen (task, agent, optional pasted context, or open a plan file); the plan
  workspace in three columns: the agent's notes and guidance, the plan list,
  and the selected step with its evidence, risk, recovery and review concerns.
- **Editing:** a step's title, objective, commands, visible validation and
  recovery commands; add a manual, check or command step after the selected
  one; move up or down; remove. Every change is a request to Rust.
- **Agents:** propose from the new-task screen; "Retry step with agent" sends
  the task, the plan, the operator's guidance and what validation and review
  found about that step, and only that step can change; "Ask agent to
  reconsider the plan"; "Ask for review" from a second agent, whose findings
  appear beside the plan and against the steps they concern. Agent output is
  never executed: it becomes a draft that has to be validated and approved.
- **Overall readiness:** `n/N ready`, whether elevation is needed, whether
  recovery is declared, and what stands between the plan and approval.
- **Approval:** a critical step is shown in full (target, commands, impact,
  recovery, validation) and approved only by typing its phrase. The sealed
  snapshot is written to `%LOCALAPPDATA%\KeyJutsu\runs\<plan>-<hash>\`, where
  `keyjutsu run` and `keyjutsu recover` can use it too.
- **Running from the desktop:** "Arm KeyJutsu" runs the snapshot through the
  Milestone 8 executor in the app's terminal, full screen. Before a critical
  step runs, the same dialog asks for the phrase again; the executor, not the
  window, compares it. After a failure the side panel offers the recovery
  plan, and rolls back only when asked.

## The execution-time gate (invariant 6)

`ExecuteOptions::critical_gate` is asked before each critical step. It
returns what the operator typed, or nothing; the executor compares it with
the phrase. A mismatch or a decline stops the run with the step not run.
`a_critical_step_is_confirmed_again_just_before_it_runs` runs a real
`Remove-Item -Recurse` against a folder three times, declined, with a wrong
phrase and with a near-miss, and checks the folder survives each time; only
the exact phrase lets it run. With the comparison removed, the test fails.

**When it asks again** (decided with the owner after the first walkthrough,
where being asked twice within a minute was redundant): the phrase typed at
approval stands for an hour. A snapshot sealed longer ago, or whose sealing
time cannot be read, has its critical steps confirmed again: in the desktop,
in the dialog just before the step runs; in the CLI, on the plain console
before the session starts, because its console is the performance
(`needs_reconfirmation`; `an_old_approval_of_a_critical_step_is_confirmed_again_before_the_run`,
which fails both if it never asks and if it always asks).

**Seen with the owner** (25 September 2026): validate, approve with the
typed phrase, run in Performance mode, the manual step, the critical dialog
again before step 4 (at the time it always asked), completion and the run
panel, on a trial plan confined to a scratch folder. The run panel's long
checkpoint path overflowed the side panel; fixed. The desktop records Git
state for the folder the app started in, while a profile that changes
location (the owner's does) starts the shell elsewhere; not yet fixed.

## Done when

"The user can fully inspect and modify what will execute without reading raw
agent chat history."

| §60 item | Status | Evidence |
| --- | --- | --- |
| Plan graph/list | List built; no graph view | Steps in execution order with number, title, shell/privilege/reversibility, readiness, risk, critical and review concerns. Branching plans are listed in graph order; the edges are not drawn. |
| Step editor, command editor | Met | `editing_a_step_sends_it_and_what_follows_back_to_validation`, `an_edit_the_plan_would_refuse_leaves_the_draft_as_it_was`, `stepFromForm` tests. |
| Guidance | Met | Guidance goes with a retry or a reconsideration, or is kept as a note. |
| Retry step | Met against recorded answers | `retrying_a_step_gives_the_agent_the_findings_and_discards_validation`. |
| Agent review | Met against recorded answers | `a_review_adds_concerns_but_changes_no_step`. |
| Validation, risk display | Met | Evidence with pass/fail glyphs and text, remaining uncertainty, proof level, risk and its reasons, separately from readiness (kit §12). |
| Dependency invalidation | Met | `change.affected` is exactly the edited step and its descendants; unaffected steps keep READY. |
| Overall readiness | Met | `a_new_plan_needs_validation_before_it_can_be_approved`; approval refused until every step is READY. |
| Critical approval | Met | `a_critical_step_is_approved_only_with_its_phrase`, plus the execution-time gate above. |

## Seen on screen

The plan workspace was built as a release app, opened on
`examples/valid/critical-reset.json` (via `KEYJUTSU_OPEN_PLAN`) and captured
from the window without sending it any input. The first capture showed four
faults, all fixed and captured again:

- empty state badges: Rust sent `null` for a missing optional field where the
  TypeScript type said "absent", so the label function returned nothing;
  the workspace types now leave missing fields out;
- the critical badge's `.critical` class matched the dialog's 720 px width,
  wrecking the row it was on;
- the wordmark's flex gap split it into "Key J utsu";
- the agent selects overflowed their column.

**Not seen:** the new-task screen, the step editor, the add-step form, the
critical dialog, a run, and the recovery panel. Reaching them means clicking,
and the driver only clicks when KeyJutsu is verifiably the foreground window;
with the owner using the machine it was not, so nothing was clicked.

## Limits

- **No live agent was called.** Propose, retry, reconsider and review are
  tested against recorded answers; the buttons call the real CLIs when used.
- **No drawn graph.** Branches and joins are only visible as order and as
  `depends_on` in the step.
- **"Disable a step" is not built.** The schema has no disabled flag;
  removing the step is the alternative.
- **Removal is refused while other steps refer to the step**, rather than
  re-linking the graph around it: the operator decides how to re-link.
- **Internal checks and preconditions are not editable in the window**; they
  are shown, and kept as they are when other fields are edited.
- **Folders and files cannot be attached as context** in the window yet; only
  pasted text. The CLI supports both.
- **The plan is not saved as a draft between sessions**; only sealed
  snapshots are written. Persistence is Milestone 14.
- **During a run, keys pressed between steps are dropped** (by Rust), and the
  disarm chord only works while a step is armed.
