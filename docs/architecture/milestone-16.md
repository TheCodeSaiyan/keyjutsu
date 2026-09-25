# Milestone 16: installer and onboarding

What was built, and what the "done when" in §60 rests on.

## What it is

- **An NSIS installer** (`pnpm desktop:build`, `target/release/bundle/nsis/`)
  carrying the desktop app, the CLI (`keyjutsu.exe`) and the elevation broker
  (`keyjutsu-broker.exe`) side by side. It installs for every user into
  Program Files, because the broker is started elevated and must sit where
  only Administrators can replace it ([deviation D29](deviations.md#d29-installed-for-every-user-and-signed-only-when-a-certificate-is-configured)).
  `scripts/stage-sidecars.mjs` builds the CLI and the broker before bundling
  and names them for Tauri.
- **No runtime to install.** The programs link the C runtime statically
  (`.cargo/config.toml`), so a machine without the Visual C++ redistributable
  runs them; the trial machine had none. The desktop app needs WebView2: the
  installer fetches it if it is missing, which Windows Sandbox showed it is
  on a stripped-down Windows 11.
- **Two questions after installing**: put `keyjutsu` on the PATH, and add
  "Open KeyJutsu here" to Explorer's folder menus. `/S` answers yes to both.
  Both are done by the installed CLI (`keyjutsu setup path|explorer
  add|remove|status`), so the installer and the product agree on what
  changed, and either can be changed later. The PATH entry is appended to the
  user's PATH, never prepended, so it cannot shadow a program already there;
  the registry value keeps its type, so `%VARIABLES%` in it are not expanded
  and frozen.
- **"Open KeyJutsu here"** starts the desktop app with `--cwd "<folder>"`, and
  terminals start there. A drive root arrives as `C:"` (Windows reads the
  `\"` in `"C:\"` as an escaped quote) and is put back.
- **The readiness scan** is `keyjutsu doctor`, and the same scan is on the
  desktop's first screen: Windows, ConPTY, each shell's staged typing and exit
  codes, agents, the broker and DPAPI storage, each reported as found.
- **The safe demo** (`keyjutsu demo`) performs three read-only commands.
- **The shell defaults to what is there**: PowerShell 7 if installed,
  otherwise Windows PowerShell, which every Windows has. Before, the CLI
  assumed PowerShell 7, and on a clean machine it would not start.
- **Uninstalling** removes the PATH entry and both Explorer menus, then the
  files. The encrypted history in the user's profile is left as the operator's
  data; `keyjutsu store clear --history --techniques` removes it.
- **CI** builds the installer on every run and keeps it as an artifact, and
  signs it when a signing certificate is configured as secrets. Only the
  signing step sees the secrets.

## Done when

"A clean Windows 11 machine can install KeyJutsu, detect supported agents and
successfully run the safe Performance Mode demo."

Shown in Windows Sandbox (25 September 2026,
`tests/e2e/install-trial/run.ps1`): Windows 11 Enterprise 26100 with no
Visual C++ runtime, no PowerShell 7 and no WebView2.

- `KeyJutsu_0.1.0_x64-setup.exe /S` exited 0 after 77 seconds, most of it
  fetching WebView2 (154.0.4258.37 afterwards). The app, the CLI, the broker
  and the uninstaller were in `C:\Program Files\KeyJutsu`; the folder was on
  the user's PATH and both Explorer menus were registered.
- A new terminal found `keyjutsu` on the PATH. `keyjutsu doctor` reported
  every check ok except "AI agents: none installed", including Windows
  PowerShell's staged typing and exit codes through ConPTY, the installed
  broker and DPAPI storage.
- `keyjutsu agents` reported every agent not installed; with a stand-in
  Codex on the PATH it reported `Codex CLI version 0.154.0, no credentials
  found` (see Limits).
- The desktop app started and was still running 8 seconds later.
- **The demo**, `keyjutsu demo --clean` in Performance mode inside a
  pseudo-console (`cli/keyjutsu/examples/demo_trial.rs`), with a key mashed
  every 15 ms: after 595 keys all three commands had been typed and run, no
  mashed key reached the shell, and after the disarm chord and `exit`
  KeyJutsu's summary read `1. Describe this computer: succeeded (exit 0)`,
  `2. Show the PowerShell version: succeeded (exit 0)`, `3. List the
  volumes: succeeded (exit 0)`.
- `uninstall.exe /S` exited 0; the files, the PATH entry and both menus were
  gone.

## Checked by breaking it

- `path_with`/`path_without` and the drive-root repair each have tests that
  fail without them.
- The first trial run had networking off, as the other Sandbox trials do; the
  installer stopped with exit code 2 because it could not fetch WebView2. The
  trial now stops at a failed installation instead of going on: that run's
  uninstall checks had "passed" on a machine where nothing was installed, and
  its app check passed for a program that did not exist.
- The next run hung in the demo, past the driver's own four-minute deadline,
  though the same driver passed on the development machine. The driver wrote
  to the pseudo-console under a lock that its reader also needed to answer
  a cursor query: when the console stopped reading input for a moment, the
  write blocked holding the lock, the reader waited, nothing drained the
  output, and the console never read again. Writes now go through one
  writer thread. That run then showed the demo had finished: Get-Volume in
  Sandbox lists no volumes, so the table the driver waited for never came.
  The driver now waits for the prompt after the last command and judges by
  KeyJutsu's own summary.

## Limits

- **Installers are not signed** until a certificate is configured, and then
  only the installer itself, not the programs inside it (D29). Windows warns
  about an unknown publisher.
- **The desktop app needs WebView2**, fetched by the installer when it is
  missing. On a machine with neither WebView2 nor a network, the installer
  stops; the CLI alone would work there, but there is no CLI-only installer.
- **Agent detection on the clean machine used a stand-in**: a real agent CLI
  needs Node or its own installer, and an account to be any use, which a
  disposable Sandbox does not have, so a `codex.cmd` answering
  `--version` as Codex does was put on the PATH. The real agents were
  detected and checked live on the development machine (see
  [agent integrations](../agent-integrations/README.md)).
- **The desktop app was checked to start and stay up** on the clean machine,
  not driven; the demo was run through the CLI.
- **No onboarding tour** beyond the readiness screen and the demo.
