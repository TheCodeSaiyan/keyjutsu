# Contributing

Thank you for wanting to help. KeyJutsu types real commands into real shells,
so the bar for changes is set by what could go wrong on someone's machine, not
by how the demo looks.

## Before you start

- Read [THREAT_MODEL.md](THREAT_MODEL.md) and the
  [state machine](docs/architecture/execution-state-machine.md). A change that
  weakens an invariant listed there needs an ADR and a reason, not just a
  pull request.
- For anything larger than a fix, open an issue first so the approach can be
  agreed before you spend time on it.

## Building and testing

```powershell
pnpm install
cargo test                       # Rust, including real shells through ConPTY
pnpm -r typecheck; pnpm -r lint; pnpm -r test
pnpm schemas:check
pnpm types:index                 # after changing a Rust type that crosses IPC
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all; pnpm format
```

`cargo test` starts PowerShell 7, Windows PowerShell 5.1 and cmd.exe in
pseudo-consoles, with the clean profile so your own profile cannot change the
result and nothing is written to your shell history. The whole Rust suite
takes a few seconds.

The desktop crate is not in the default workspace members, because it embeds
the built frontend at compile time: run `pnpm --filter @keyjutsu/desktop build`
before `cargo build -p keyjutsu-desktop`.

## What a change needs

- **A test that fails without it.** For a bug fix, write the test first and
  watch it fail. A test that passes either way is worse than none, because it
  looks like cover.
- **Tests on real shells for anything touching the terminal.** Mocked
  pseudo-consoles have hidden every interesting bug found so far; see the list
  in [milestones-0-2.md](docs/architecture/milestones-0-2.md).
- **The threat model updated** if the change touches a trust boundary.
- **A deviation recorded** in [deviations.md](docs/architecture/deviations.md)
  if it departs from the specification.
- **A new dependency must be GPL-3.0-compatible**: MIT, Apache-2.0, BSD,
  Zlib, ISC and MPL-2.0 are; GPL-2.0-only and proprietary licences are not.
- **No new dependency to save a dozen lines**, and none at all in
  `keyjutsu-execution`, which is kept free of I/O on purpose.
- **No `unsafe`** beyond the one documented call. The lint denies it.

## Writing style

Documentation is in British English. Say why, not only what; use a real
figure rather than an adjective; say plainly what something does not do.

## Contributions and licensing

KeyJutsu is licensed GPL-3.0-only ([LICENSE](LICENSE)), and by contributing
you agree your contribution is licensed the same way. Sign off each commit
(`git commit -s`) to certify the
[Developer Certificate of Origin](https://developercertificate.org/).
