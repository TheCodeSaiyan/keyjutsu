# 0011: A separate, narrowly scoped elevation broker

Status: proposed, for Milestone 10. Recorded now so nothing built before then
makes it harder.

## Proposal

- The app and CLI always run unelevated. Administrator steps go to
  `keyjutsu-broker`, a separate binary started elevated once per performance
  (one UAC prompt, before arming, never mid-performance).
- The broker listens on a named pipe whose ACL admits only the launching
  user's logon SID, and checks the client's process identity on connect.
- It accepts one kind of request: *run approved step N of snapshot H*, with
  the approved snapshot supplied at start-up. It recomputes the step hash
  itself and refuses anything that does not match. There is no endpoint that
  takes a command string.
- The protocol is versioned; a version mismatch is a refusal, not a
  negotiation.
- The broker exits when the performance ends, disarms or fails.

## What this rules out now

No code before Milestone 10 may run anything elevated, and there is no
temporary elevated `execute(string)` path "to be secured later" (§59). The
readiness scan reports the broker as not yet built.

## Open question

Whether the elevated step runs in its own pseudo-console owned by the broker,
mirrored into the visible terminal, or in the visible terminal's shell by some
other route. §15 requires the visible terminal and the real execution to
correspond, which the first option has to demonstrate rather than assume.
