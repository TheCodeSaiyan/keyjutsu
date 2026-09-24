# 0010: Canonical JSON and SHA-256 for plan and step hashes

Status: proposed, for Milestone 4. Recorded now because the plan schema's
`step_hash` and `snapshot_hash` fields depend on it.

## Proposal

- A step's hash is SHA-256 over the RFC 8785 (JSON Canonicalisation Scheme)
  serialisation of the step's execution-relevant fields: commands, shell
  binding, working directory, target, privilege, tool requirements,
  preconditions, validation, recovery, network and artifact contracts, and the
  hashes of the steps it depends on. Titles and prose are excluded, so
  rewording a description does not invalidate an approval, while any change to
  what runs does.
- The snapshot hash covers the ordered step hashes, the edges, the
  environment fingerprint and the schema version.
- Including dependency hashes in each step's hash is what makes invalidation
  flow downstream (§11): change step 3 and steps 4, 5 and 7, which depend on
  it, get new hashes and lose their approval automatically.

## Open questions for Milestone 4

- Whether the environment fingerprint belongs in step hashes or only in the
  snapshot. Per step is stricter but invalidates more on unrelated drift.
- Which JCS implementation to use, or whether to write the canonicaliser
  (number formatting is the part most often got wrong).
