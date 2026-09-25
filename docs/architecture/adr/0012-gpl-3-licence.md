# 0012: KeyJutsu is licensed GPL-3.0-only

Status: accepted, 24 September 2026. Supersedes the Apache-2.0 licence in
specification §1.

## Context

The specification named Apache-2.0. The project owner asked for tighter terms
that still let KeyJutsu use open-source tools. "Tighter" here means nobody can
take KeyJutsu, change it and ship the result closed. Apache-2.0 allows exactly
that.

## Options considered

| Licence | Closed forks | Still OSI open source | Fit |
| --- | --- | --- | --- |
| Apache-2.0 | allowed | yes | what the specification said; not tighter |
| MPL-2.0 | changed files must stay open; can be embedded in closed products | yes | tighter, but a wrapper around KeyJutsu can stay closed |
| **GPL-3.0** | **not allowed when distributed** | **yes** | **KeyJutsu is a distributed desktop app, which is what the GPL covers** |
| AGPL-3.0 | not allowed, including over a network | yes | adds nothing while KeyJutsu has no hosted service (§54) |
| PolyForm Noncommercial, FSL | commercial use needs permission | no | contradicts "open source" in §1 |

## Decision

GPL-3.0-only. "Only" rather than "or later", so a future GPL version cannot
change KeyJutsu's terms without the owner choosing it.

## Dependencies

Checked on 24 September 2026 with `cargo metadata` and `pnpm licenses`: about
470 dependencies, all MIT, Apache-2.0, BSD, Zlib, ISC, Unicode-3.0, 0BSD, CC0
or the Unlicense, apart from five MPL-2.0 crates. Every one can be combined
into a GPL-3.0 program: Apache-2.0 is compatible with GPL version 3
(though not version 2), and MPL-2.0 allows combination with the GPL unless a
file opts out, which none of these do. Their own licence notices must still
ship with binaries; that belongs to the release pipeline (M16), with the SBOM.

A dependency added later needs the same check. A GPL-2.0-only or proprietary
dependency could not be used.

## Consequences

- Contributions come in under GPL-3.0-only, certified with DCO sign-off.
  Relicensing later would need every contributor's agreement; if dual
  licensing is ever wanted, a contributor licence agreement has to be in
  place before outside contributions arrive, not after.
- The owner, as sole copyright holder today, can still offer KeyJutsu under
  other terms.
