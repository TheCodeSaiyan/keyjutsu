# Milestone 6: agent integration

What was built, and what each "done when" in §60 rests on. The adapters and
their limits are described in [docs/agent-integrations](../agent-integrations/README.md).

## What it is

A new crate, `keyjutsu-agent`:

- **`agents`**: the five agents, their read-only invocations and the
  compatibility matrix.
- **`detect`**: what is installed, its version, and whether credentials exist
  (by file presence only, never reading them).
- **`context`**: the manifest, redaction, and the folder warning.
- **`prompt`**: proposal, revision, review and repair prompts, each restating
  the ground rules.
- **`extract`**: the final message from each agent's output format, and the
  JSON document inside it.
- **`review`**: the review format, which cannot name steps that do not exist.
- **`session`**: propose, revise one step, revise the plan, review; the repair
  loop, identity stamping and provenance.

CLI: `keyjutsu agents`, `keyjutsu agents check --live`, and
`keyjutsu plan propose | revise | review`, each of which sends nothing without
`--send`.

## Done when

| §60 criterion | Status | Evidence |
| --- | --- | --- |
| Detect installed agents and versions | Met | `keyjutsu agents` found Codex 0.154.0, Claude Code 2.1.282, Gemini 0.32.1 and Copilot 1.0.78 on the development machine; `reads_the_versions_these_clis_actually_print`. |
| Select a primary agent | Met | `--agent` on every command. |
| Provide task and context | Met | `a_secret_in_pasted_context_never_reaches_the_agent`, `a_folder_is_not_read_but_its_secret_looking_files_are_named`. |
| Obtain a structured plan proposal | Met against recorded answers | `a_proposal_is_accepted_and_stamped_with_the_agent_that_really_wrote_it`, `codexs_answer_is_read_from_its_output_file`. **Not yet against a live agent.** |
| Request step revision and full-plan revision | Met against recorded answers | `a_step_revision_changes_that_step_only_and_discards_validation`. |
| Request an independent review | Met against recorded answers | `a_review_challenges_steps_but_changes_nothing`. |
| Capture provenance | Met | authored, revised, challenged and validated events in the KeyJutsu-owned `provenance`. |
| State-changing suggestions remain proposals | Met | agents run in their read-only modes; nothing they return is executed; everything goes through `parse_proposal`. |

## Checked by breaking it

- Removing the identity stamp let a document claiming "Codex 0.1" keep that
  identity when Claude Code wrote it; the identity test caught it.

## Limits

- **No live agent was called while building this.** Doing so would have sent
  requests on the owner's accounts without their say. The adapters follow the
  CLIs' help and documented output shapes; `keyjutsu agents check --live` is
  the first real test, and its result should go into the matrix.
- **Cursor is unverified** (deviation D17).
- **Invariant 2 (secrets never enter agent context) is partial.** Pattern
  redaction covers the common shapes; a folder the agent investigates is read
  by the agent, not by KeyJutsu.
- **No desktop UI for this yet.** The plan workspace is Milestone 7.
