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
pnpm docs:check                  # the docs against the CLI, links, and the house style
pnpm release:gates               # the eight release gates, by name
pnpm types:index                 # after changing a Rust type that crosses IPC
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all; pnpm format
```

`cargo test` starts PowerShell 7, Windows PowerShell 5.1 and cmd.exe in
pseudo-consoles, with the clean profile so your own profile cannot change the
result and nothing is written to your shell history. The whole Rust suite,
471 tests, takes a few minutes, most of it the real shells and the property
tests.

The desktop crate is not in the default workspace members, because it embeds
the built frontend at compile time: run `pnpm --filter @keyjutsu/desktop build`
before `cargo build -p keyjutsu-desktop`.

## What a change needs

- **A test that fails without it.** For a bug fix, write the test first and
  watch it fail. A test that passes either way is worse than none, because it
  looks like cover.
- **Tests on real shells for anything touching the terminal.** Mocked
  pseudo-consoles have hidden every interesting terminal bug found so far.
- **The threat model updated** if the change touches a trust boundary.
- **A new dependency must be GPL-3.0-compatible**: MIT, Apache-2.0, BSD,
  Zlib, ISC and MPL-2.0 are; GPL-2.0-only and proprietary licences are not.
- **No new dependency to save a dozen lines**, and none at all in
  `keyjutsu-execution`, which is kept free of I/O on purpose.
- **No new `unsafe`.** The lint denies it everywhere except four modules
  that call Windows directly (the console, DPAPI, the broker's pipe and the
  elevation check), each with the invariant it relies on written beside it.

## Writing style

Documentation is in British English. Say why, not only what; use a real
figure rather than an adjective; say plainly what something does not do.
Output shown on a page is output a command really printed, and screenshots are
the real app. Both come from a clean Windows 11, not anyone's own machine:
`tests/e2e/docs-capture/run.ps1` installs KeyJutsu in Windows Sandbox, runs
each documented command there and drives the app for the screenshots. Pages follow the
layout in [the documentation index](docs/README.md): a guide does one job,
opens with what you'll have at the end, and numbers its steps.
`pnpm docs:check` fails on a `keyjutsu` command or option that doesn't
exist, a broken link or heading, a page nothing links to, and the words the
house style avoids. It also fails, anywhere in the repository, on a
reference to a numbered section of a planning document or to a build
milestone: a reader has neither, so say what the reference stood for
instead.

## Releasing

Versions are `MAJOR.MINOR.PATCH`. The version is written in several places
that have to agree (the workspace `Cargo.toml`, `tauri.conf.json`, the
`package.json` files and the installer's name in the install guides), so it's
set with one command and checked in CI:

```powershell
node scripts/version.mjs set 0.2.0     # every place, and Cargo.lock
git commit -am "Release 0.2.0"
git tag -a v0.2.0 -F notes.md           # what changed, for the release page
git push origin main v0.2.0
```

The tag starts `.github/workflows/release.yml`. It refuses a tag that
disagrees with the version in the code, builds the installer, signs it and
every program in it, installs it on a clean runner, runs `keyjutsu doctor`,
uninstalls it and checks nothing was left, then runs the portable build from
a folder where nothing was installed, and only then publishes. The
release page gets the tag's notes, the one-line installer
(`scripts/install.ps1`, which checks the download against `SHA256SUMS` and its
signature before running it), and whether this release is signed. Running the
workflow from the Actions page builds and checks everything and publishes
nothing, and says so in its title.

The portable build, `KeyJutsu_X.Y.Z_x64-portable.zip`, is packed by
`scripts/portable.ps1` from the programs the installer has just installed
and checked, so it's signed exactly when they are. It holds the app and the
CLI, and a `README.txt` saying what it doesn't do. It leaves out the
elevation broker on purpose: the broker runs as Administrator, so it's only
installed where only Administrators can replace it, and a portable folder is
usually one the user can write to. The release run checks the zip doesn't
carry it.

The winget manifests are written by `scripts/winget.mjs` from the installer
being published, with its hash, and kept as the `winget-manifests` artifact
of the release run; nothing is sent to winget automatically. To submit a
release, check them with `winget validate --manifest <folder>` and open a pull
request adding the folder to
[microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs) under
`manifests/t/TheCodeSaiyan/KeyJutsu/`. winget downloads the installer from the
release page, so the repository has to be public for anyone else to install
it that way.

Signing uses Azure Trusted Signing through GitHub's OIDC token, so there is no
certificate file to keep. It's switched on by the repository's `release`
environment having these secrets: `ARTIFACT_SIGNING_ACCOUNT`,
`ARTIFACT_SIGNING_ENDPOINT`, `ARTIFACT_SIGNING_PROFILE`, and `AZURE_CLIENT_ID`,
`AZURE_TENANT_ID` and `AZURE_SUBSCRIPTION_ID` for the federated login. With
none of them, releases are built unsigned and their page says so; with only
some, the build refuses rather than half-signing.

## Contributions and licensing

KeyJutsu is licensed GPL-3.0-only ([LICENSE](LICENSE)), and by contributing
you agree your contribution is licensed the same way. Sign off each commit
(`git commit -s`) to certify the
[Developer Certificate of Origin](https://developercertificate.org/).
