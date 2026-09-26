# 0010: Canonical JSON and SHA-256 for plan and step hashes

Status: accepted. It was decided first because the plan schema's hash
fields depended on it.

## Decision

Every hash is SHA-256 over the RFC 8785 (JSON Canonicalization Scheme) form of
a JSON object that starts with a `kind` naming the hash's own format
(`keyjutsu.step-hash/1`, `keyjutsu.snapshot/1`, `keyjutsu.fingerprint/1`), so a
hash of one kind can never stand in for another. The canonicaliser is written
here, not taken from a crate (`crates/keyjutsu-plan/src/canonical.rs`); it is
about 100 lines and passes RFC 8785's own number and key-ordering vectors.

**A step hash** covers:

- every field of the step except `title`, `objective` and `reason`, so
  rewording costs no approval while any change to what runs does;
- the plan-wide context every step relies on: schema version, target,
  requirements and environment assumptions;
- for each incoming edge, the source step's hash and the edge's condition;
- the hash of every step in `depends_on`.

Chaining predecessor hashes is what makes invalidation flow downstream: change
step 3 and steps 4, 5 and 7 get new hashes too. `diff` computes
the same "affected" set independently; a property test across 500 random
edit sequences holds the two answers together.

**A snapshot hash** covers everything: the plan as approved (wording
included), the step hashes, the approvals, the environment fingerprint's hash
and the seal time. It is the answer to "what exact plan was executed?".

**The environment fingerprint** is kept out of step hashes. Putting it in would
withdraw every approval whenever anything on the machine changed; instead
drift is compared separately and mapped to the steps it actually touches
(`affected_by_drift`): an OS or build change affects every step, a shell
change the steps bound to that shell, a tool change the steps that name it.

## Limits, stated plainly

- **The hashes are unkeyed.** They detect accidental and naive edits to a
  stored snapshot; anyone able to write the file can edit it and recompute
  every hash. The typed confirmation on critical steps is checked on load
  independently of the hash, but its phrase is derivable from the step's
  title, so it only stops a forger who does not bother. Tamper resistance
  against a local attacker needs a keyed MAC under a DPAPI-protected key: the
  key belongs to encrypted storage and the verifier that
  matters is the elevated broker. This was later closed
  differently: sealing records the snapshot hash in the DPAPI-keyed store,
  and a snapshot without that record is refused.
- **Tool versions are not in the fingerprint yet.** Paths are; asking a tool
  its version is validation's job.
