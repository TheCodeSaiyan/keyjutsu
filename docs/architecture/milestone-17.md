# Milestone 17: security hardening

What was done, and what the "done when" in §60 rests on.

## Done when

"All security release gates pass and the threat model matches the
implemented architecture."

- **The gates.** §57's eight gates are named test lists in
  `scripts/release-gates.mjs` (`pnpm release:gates`, and in CI after the full
  suite). A missing test fails its gate, so a rename cannot drop one
  silently; the first run showed that, reporting seven unit tests as missing
  until the runner matched names inside modules. All eight pass: broker
  security 10/10, plan integrity 13/13, secret handling 8/8, execution state
  machine 16/16, schema validation 7/7, supported-shell compatibility 8/8,
  critical rollback 10/10, credential boundary 6/6.
- **The threat model** was read row by row against the code. Stale claims
  were corrected (the introduction still described Milestone 6; two rows said
  fuzzing was to come; the compromised-renderer row implied it could not
  approve, which it can). Every change below has its row, and a new "Not
  covered" section gathers what the document does not claim.

## What changed

**Plan tampering.** A sealed snapshot's hashes are consistent for anyone who
can recompute them, so a snapshot edited and re-sealed with KeyJutsu's own
library passed every check. Sealing now also records the snapshot hash in the
DPAPI-keyed encrypted store, and `run` and `recover` refuse a snapshot this
Windows account never approved on this machine. Checkpoints are recorded the
same way on every save and refused if they differ, which closes deviations
D14 and D20. Random edits to a stored snapshot (3,000 per run) are either
refused or leave exactly what was approved.

**Failure injection.**
- The checkpoint that marks a step as started could fail to be written, and
  the step ran anyway, so a crash would have left it looking as though it
  never ran. Now the step does not start.
- A broker that dies mid-step leaves the step in doubt and stops the run;
  that already held and is now tested.
- Parallel CLI tests lost approvals, which exposed a real race: two
  processes opening a new store at once each made a key, and the last one
  written made the other's records unreadable. They also shared a temporary
  file name. The key is now linked into place only if none exists. The
  regression test failed before the fix and passed three times after.

**Fuzzing** (`proptest`, in the normal suite; see D30):
- The terminal mark scanner, with output built from real marks and
  near-misses: without the nonce nothing is trusted and no byte is lost, and
  how output is split into reads changes nothing. The second property found
  a real bug: an unfinished `ESC ] 133 ;` followed by enough output made the
  prompt's real mark part of one long sequence, so any command could make a
  step hang until its timeout. An ESC now ends a sequence, as in a real
  terminal, and nothing is searched past 4 KiB. The named regression test
  fails on the old scanner.
- The broker: 3,000 sequences of genuine, forged, mutated and garbage
  requests. A step runs only for a client that proved the secret, and only
  the approved Administrator step at its approved hash. A separate test shows
  the genuine sequence does run, so "nothing ran" cannot pass for a broker
  that runs nothing. A frame claiming 4 GiB is refused before anything is
  allocated.
- Sealed snapshots, as above; plans and proposals were already property-tested.

**IPC attacks.** Beyond the broker, the desktop's IPC surface was reviewed:
thirty KeyJutsu commands, no file-system or process plugin, and a CSP
allowing only the app's own scripts. A compromised renderer can do whatever
the window can, approving and running included, so the defence is keeping
script out of it: nothing in the frontend renders plan or terminal text as
HTML, and xterm.js has no link handler or proposed API enabled. This is now
stated in the threat model rather than implied away.

**Hidden characters.** A command containing a bidirectional override, a
zero-width character or another invisible one reads differently on the
approval screen from how it runs ("Trojan Source", CVE-2021-42574). Such
commands are now a structural problem, refused whoever wrote them: an agent,
an imported Technique or the operator's own edit. Technique parameter values
may not contain them either.

**Imported Technique attacks.** A crafted export claiming known-good
environments, a validation date, "not imported" or KeyJutsu's own state
arrives with all of it cleared. Hidden characters in its commands or
defaults are refused, and the refusal now says why instead of "1 problem(s)".
A loose parameter pattern can let a value add flags (`none -Recurse -Force`);
nothing stops that at the value, and the test shows what does: the plan is
judged by the command it makes, and recursive deletion is held for its typed
confirmation.

**Secret leakage.** Only pasted context had been redacted before reaching an
agent. The task, the operator's guidance for a revision, what validation
found and the plan itself (an edited step may hold a value the operator
typed) went unredacted. All of it now passes through the same redaction, and
one test sends a token down every route.

## Checked by breaking it

Where a fix loosens nothing, the test was run against the old code: the
scanner test and the store race test both failed before their fixes. For the
approval, checkpoint and redaction checks, removing the check would briefly
disable a security control, so those tests prove themselves with input that
must be refused: a re-sealed snapshot, a moved approval record, an edited
checkpoint, an unwritable checkpoint and a real-format token down every
route. Each would pass unrefused if its check were missing.

## Limits

- **Fuzzing is not coverage-guided** (D30).
- **Another process running as the same user** can forge approvals through
  DPAPI; the threat model says so. It could also just run commands.
- **One previous checkpoint is also accepted**, the cost of surviving a crash
  between recording a save and writing it.
- **The desktop has no screen for a refused snapshot**: it runs what it holds
  in memory, and only its recovery reads a checkpoint back.
- **The CLI end-to-end tests** still share one locked writer between reading
  and writing the pseudo-console, the shape that hung the Milestone 16 demo
  driver. They have not hung; if they do, they need its one-writer thread.
