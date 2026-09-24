# Milestone 4: immutable approval

What was built, and what each "done when" in §60 rests on.

## What it is

In `keyjutsu-plan`, still without I/O:

- **`canonical`**: RFC 8785 canonical JSON.
- **`hash`**: step hashes, snapshot hashes, environment fingerprints, drift
  and the steps drift affects. How and why:
  [ADR 0010](adr/0010-plan-hashing.md).
- **`approval`**: the operator's approvals, bound to step hashes; the typed
  confirmation for critical steps; sealing into an `ApprovedSnapshot` that has
  no mutating methods and re-checks every hash when loaded.

In `keyjutsu-core`, the I/O: collecting the fingerprint (Windows build,
architecture, each shell's path and version, where each named executable
resolves) and an RFC 3339 clock.

In the CLI:

```powershell
keyjutsu plan hash plan.json                        # what approvals bind to
keyjutsu plan approve plan.json --out snapshot.json # seal, if nothing is critical
keyjutsu plan approve plan.json --out snapshot.json --confirm remove-data="REMOVE LOCAL DOCKER DATA"
keyjutsu plan verify snapshot.json --environment    # intact? drifted?
keyjutsu plan diff old.json new.json                # what needs revalidation
```

A critical step that has not been confirmed is shown the way §28 asks:
exact command, target, impact, reversibility, recovery, and the phrase to
type. Whole-plan approval never covers it.

## Done when

| §60 criterion | Status | Evidence |
| --- | --- | --- |
| Any execution-relevant mutation after approval invalidates authorisation | Met | `changing_a_step_invalidates_it_and_everything_after_it` (§11's own example: steps 3, 4, 5 and 7 lose approval, 6 keeps it); `a_new_branch_condition_invalidates_its_target_and_what_follows`; `a_plan_wide_requirement_invalidates_every_step`; `rewording_a_step_keeps_every_approval`; the property test `hashes_and_diff_agree_on_what_a_change_affects`, 500 cases. |
| "What exact plan was executed?" has a deterministic, immutable answer | Met | `a_sealed_snapshot_round_trips_exactly` (same approvals at the same moment, same hash; the stored form is byte-stable); `ApprovedSnapshot` has no setters. |
| Draft versus approved snapshot | Met | `ApprovalBook` over a draft; `seal` refuses anything not approved at its current hash. |
| Environment fingerprinting | Met, without tool versions | `EnvironmentFingerprint`, `drift`, `affected_by_drift`; `drift_puts_only_the_affected_steps_in_question`; `keyjutsu plan verify --environment`. |
| Approval state | Met | `StepApproval::{Approved, NotApproved, Invalidated}`. |

Tamper detection: `any_edit_to_a_stored_snapshot_is_refused` edits ten
different parts of a stored snapshot (a command, a title, an approval's hash,
a missing approval, the recorded hashes, the snapshot hash, the fingerprint,
the seal time, an added edge, a smuggled KeyJutsu state section); every one is
refused. The CLI test does the same through `keyjutsu plan verify`.

## Checked by breaking it

- Dropping predecessor hashes from step hashes failed the invalidation test
  and the hash/diff property.
- Removing the load-time check for a critical step's typed confirmation was
  at first *not* caught, because the snapshot hash caught the edit first. The
  test now plays a forger who recomputes the hash, which only the explicit
  check can stop; with the check removed it fails.

## Limits

- **Unkeyed hashes** (deviation D14): integrity against accident, not against
  a local attacker who can write the file. The keyed MAC waits for the
  DPAPI-protected key of Milestone 14 and the broker of Milestone 10.
- **Critical is the agent's word, for now** (deviation D13): a step is
  critical if the agent proposed it so or KeyJutsu's recorded assessment says
  so. KeyJutsu's own risk rules, which catch an agent that under-states risk,
  arrive with validation in Milestone 5.
- **Nothing executes snapshots yet.** Milestone 8 runs them; Milestone 5
  adds the readiness gate (no step may be armed unless Ready).
