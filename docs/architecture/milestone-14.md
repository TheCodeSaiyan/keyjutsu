# Milestone 14: persistence and Techniques

What was built, and what the "done when" in §60 rests on. The store's design
is [ADR 0016](adr/0016-encrypted-file-store.md).

## What it is

- **An encrypted local store**: AES-256-GCM records under a random key that
  only this Windows user on this machine can unprotect (DPAPI).
- **Session history.** When `keyjutsu run` ends, the session is recorded:
  task, agent, the sealed snapshot exactly as approved, the checkpoint, the
  outcome and the Git report. `--ephemeral` records nothing.
  `keyjutsu history list | show | recheck`.
- **Reopening compares environments.** `keyjutsu history recheck` verifies
  the recorded snapshot again and compares this machine with the one it was
  approved on, naming the steps whose approval relied on something that has
  changed (§36).
- **Techniques.** `keyjutsu technique promote <session> --name … --param
  service_name=Winmgmt` turns a completed session into revision 1: the value
  becomes `{{kj:service_name}}` everywhere in the steps, and the Technique
  keeps where it came from (session, agent, reviewers, when it was last
  validated) and the environment it worked on.
- **Using a Technique makes a draft, never a run.** `keyjutsu technique use`
  fills the parameters, writes an ordinary draft plan, and compares this
  machine with the environments it worked on, naming the steps that require
  revalidation. The draft is validated and approved like any other.
- **Parameters are data, not code.** Each value must match the parameter's
  pattern, and whatever the pattern says, a value can never contain quotes,
  `$`, `;`, `|`, `&`, redirection, backticks, braces, brackets or line breaks.
- **Revisions.** An adapted template (after revalidation, or an agent's
  adaptation through `keyjutsu plan revise`) is saved with `technique revise`
  as the next revision; earlier revisions are kept exactly as they were.
- **Sharing.** `technique export` leaves out the originating session and this
  machine's environments; `technique import` checks the template against the
  schema and marks it imported, with no known-good environment, so every
  step must be validated and approved here.
- **Storage management:** `keyjutsu store clear --history --techniques
  --artifacts`.

## Done when

"A successful session can become a reusable Technique, be reopened later on
a changed environment, detect incompatibility and require
revalidation/adaptation."

`a_successful_session_becomes_a_technique_that_is_held_for_revalidation_on_a_changed_machine`:
a plan is validated, approved with this machine's fingerprint and run for
real; the session is recorded in the encrypted store and read back
identical; it is promoted with a parameter (a Docker-style `{{.Field}}` in a
command is left alone); used again on this machine it shows no drift and
validates READY; on a machine whose PowerShell is not the version it worked
on, both steps are named as requiring revalidation, and so does rechecking
the recorded session.

`an_incompatible_technique_is_adapted_as_a_new_revision_and_the_old_one_is_kept`:
a Technique that needs a PowerShell this machine does not have validates
BLOCKED; the adapted template becomes revision 2, which validates READY;
revision 1 is unchanged and cannot be overwritten.

## Checked by breaking it

- Encrypting without binding the record's name: a record copied to another
  id was read as that id. The store test fails.
- Removing the hard floor on parameter values: a pattern of `.*` let
  `$(Get-Date)` into a command. The parameter test fails.
- Exporting without clearing the known-good environments: this machine's
  build number was in the shared file. The sharing test fails.

## Also found

`{{name}}` placeholders would have collided with real commands: the sample
plans already use `docker version --format '{{.Server.Version}}'`, which
would have been refused as an unfilled placeholder. Placeholders are
`{{kj:name}}`.

Tests that run `keyjutsu run` would now have written to the operator's real
history. The store moves with `KEYJUTSU_STORE`, and the CLI tests set it to a
scratch folder; the operator's store was checked to be untouched.

## Limits

- **No desktop screens for history or Techniques yet**; the CLI has them.
  Desktop runs are not yet recorded in the history.
- **Replay is reopen and recheck**, not re-running the recorded terminal
  output: KeyJutsu does not record terminal output (a credential could be in
  it only masked, but a command's output could hold anything).
- **Plan revisions during editing are not kept**: the history holds the final
  snapshot, its provenance and the run, not every draft in between.
- **"Ask Agent to Adapt" is the existing `keyjutsu plan revise`** on the
  draft; there is no one-step adapt command yet.
- **Parameters apply to step text and titles**, not to plan-level
  assumptions or requirements.
- **An ephemeral session leaves its run folder** (snapshot, checkpoint) next
  to the snapshot file the operator gave; only the history record is skipped.
