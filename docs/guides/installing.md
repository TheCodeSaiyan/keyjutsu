# Installing

**At the end:** KeyJutsu is installed, `keyjutsu` runs from any new terminal,
and `keyjutsu doctor` has told you whether this machine is ready.

## Before you start

- **Windows 11, x64.** KeyJutsu drives Windows' own pseudo-console (ConPTY)
  and nothing else, so there is no macOS or Linux build.
- **An Administrator password, once.** The installer puts KeyJutsu in Program
  Files for every user, and that needs one UAC prompt. The reason is the
  elevation broker: it is started as Administrator, so it has to live where
  only Administrators can replace it.
- **The installer,** `KeyJutsu_0.1.0_x64-setup.exe`, from the
  [latest release](https://github.com/TheCodeSaiyan/keyjutsu/releases/latest).
  Until the first release is published, build it: `pnpm desktop:build`, from
  a clone with Rust and Node installed, leaves it in
  `target/release/bundle/nsis/`.

You don't need PowerShell 7, the Visual C++ runtime or an AI agent first.
KeyJutsu falls back to Windows PowerShell, which every Windows has; the
programs carry their own C runtime; and agents can come later.

## Steps

### 1. Run the installer

The quickest way is one line in PowerShell:

```powershell
irm https://github.com/TheCodeSaiyan/keyjutsu/releases/latest/download/install.ps1 | iex
```

It downloads the installer, checks it against the release's `SHA256SUMS`
and refuses if it differs, checks its signature, and then runs it: the same
installer, with the same questions. Or download the installer yourself and
double-click it. The installer isn't signed yet, so Windows shows "Windows
protected your PC": choose **More info**, then **Run anyway**. Then approve the
UAC prompt.

Each release also lists everything it ships in `KeyJutsu_VERSION_sbom.cdx.json`
(a CycloneDX software bill of materials): every Rust crate and npm package,
with its version, its licence and the hash the lock file pins, checked
against `SHA256SUMS` like the installer.

If the machine has no WebView2 runtime, the installer fetches it from
Microsoft, which needs the network. Windows 11 normally has it already; the
clean Windows 11 this was tested on didn't, and fetching it took most of
the 77 seconds the install took.

### 2. Answer the two questions

- **Add the keyjutsu command to your PATH?** Say yes unless you'd rather run
  it by its full path. It is added at the end of your own PATH, never the
  front, so it can't hide a program you already have.
- **Add "Open KeyJutsu here" to Explorer's folder menus?** Right-clicking a
  folder, or the empty space inside one, then opens the desktop app with its
  terminal already in that folder.

A silent install, `KeyJutsu_0.1.0_x64-setup.exe /S`, answers yes to both.

### 3. Open a new terminal

A PATH change reaches terminals opened after it, not the one that was already
open.

### 4. Check the machine

```powershell
keyjutsu doctor
```

On a clean Windows 11 with two agents installed and nothing else, this is what
you see:

```text
KeyJutsu 0.1.0

  [ok  ] Windows                          Windows 11 Enterprise 24H2
  [ok  ] Architecture                     x64
  [ok  ] ConPTY                           a pseudo-console started and a shell drew its prompt through it
  [ok  ] Windows PowerShell 5.1 staged input staged typing and exit codes both work
  [ok  ] Command Prompt staged input      staged typing works; this shell cannot report exit codes
  [warn] PowerShell 7                     not installed: Windows PowerShell 5.1 is used instead, and a plan written for PowerShell 7 cannot run
                                          Get PowerShell 7: https://learn.microsoft.com/powershell/scripting/install/install-powershell-on-windows
  [warn] Git                              not installed: a run in a Git repository cannot tell its changes from yours, or run isolated
                                          Get Git for Windows: https://git-scm.com/downloads/win
  [ok  ] WebView2                         154.0.4258.37: what the desktop app draws its window with
  [ok  ] Telemetry                        off: KeyJutsu has no telemetry, crash reporting or remote diagnostics
  [ok  ] AI agents                        Codex CLI, Claude Code
  [ok  ] Elevation broker                 installed: Administrator steps run through it, after one UAC prompt before the run
  [ok  ] Encrypted storage                history is encrypted with a key only this Windows account can unlock (DPAPI)
```

The desktop app runs the same checks the first time it opens:

![KeyJutsu's first screen on a clean Windows 11: the readiness checks. Windows 11, x64, ConPTY, and staged input for Windows PowerShell 5.1 and Command Prompt are ticked. PowerShell 7 and Git are marked with an exclamation mark as not installed, each with a link: Get PowerShell 7, Get Git for Windows. WebView2, telemetry off, two AI agents found, the elevation broker and encrypted storage are ticked. Below them, Start using KeyJutsu and Try the safe demo.](../images/first-run.png)

Every line comes from actually trying it. "Staged input" means a
shell was started in a pseudo-console and KeyJutsu typed into it, so an `ok`
there means performances will work in that shell.

The `warn`s are expected on a new machine, and each says where to get what's
missing, from its maker: `doctor` prints the address, and the app shows it as
a link you can click. KeyJutsu doesn't install these itself, because each is
yours to choose and keep up to date:

- **PowerShell 7.** Windows PowerShell 5.1 comes with Windows and does the
  work without it; a plan written for PowerShell 7 needs it.
- **Git.** Without it, a run in a Git repository can't tell its changes from
  yours, or run on a branch of its own.
- **An AI agent**, if you have none yet. KeyJutsu works with the agents you
  already use rather than bringing its own. [Agents](../agent-integrations/README.md) lists which
  work, and `keyjutsu agents` says where to get each.

The desktop app also needs Microsoft's WebView2 runtime to open. The installer
fetches it if it's missing, which needs a network connection; `doctor`
reports it, with where to get it, if it's not there.

### 5. See which agents it found

```powershell
keyjutsu agents
```

Each supported agent is listed as installed, with its version and whether it
looks signed in, or as not installed, with where to get it. Signed in is
judged only by whether the agent's credential file exists; the file is never
opened.

## What the installer changed

- `C:\Program Files\KeyJutsu`: the desktop app, `keyjutsu.exe` and
  `keyjutsu-broker.exe`.
- Your PATH, and the two Explorer menus, if you said yes. Both belong to your
  Windows account only, and you can change your mind later:

  ```powershell
  keyjutsu setup path status
  keyjutsu setup explorer remove
  ```

## Trying it without installing

Each release also has `KeyJutsu_0.1.0_x64-portable.zip`: the app and the
CLI, with nothing to install. Unzip the folder anywhere you can write to and
run `keyjutsu-desktop.exe`, or `keyjutsu.exe` from a terminal.

It can't run Administrator steps. Those go through KeyJutsu's elevation
broker, which runs as Administrator, so it's only installed where only
Administrators can replace it; in a folder you can write to, anything running
as you could swap it. A plan with Administrator steps is still validated,
and refused before it runs.

It doesn't add itself to your PATH or to Explorer, and has no Start menu entry
or uninstaller; `keyjutsu setup path add` and `keyjutsu setup explorer add`
do the first two for your account if you want them. It keeps its history in
the same place as an installed copy, so deleting the folder leaves that
behind until you run `keyjutsu store clear`.

## Uninstalling

**Settings**, then **Apps**, then **Installed apps**, then **KeyJutsu**, then
**Uninstall**, or `uninstall.exe /S` in the install folder. The PATH entry and
the Explorer menus go before the files. What KeyJutsu keeps in
`%LOCALAPPDATA%\KeyJutsu` stays, because it's your history rather than part
of the program; `keyjutsu store clear` removes the history, Techniques and
downloads first, if you want them gone. [Privacy](../../PRIVACY.md) lists
everything in there.

## If something's wrong

- **`keyjutsu` is not recognised.** The terminal was open before the install,
  or you said no to the PATH. Open a new one, or run
  `keyjutsu setup path add` by its full path.
- **A `doctor` line says `FAIL`.** It names what failed, and `doctor` exits
  with 1 so a script can tell. ConPTY failing means performances can't work
  at all on this machine; a shell failing means only that shell is out.
- **The installer stops with nothing installed.** On a machine without WebView2
  and without a network, it can't fetch the runtime and gives up. Only the app
  needs WebView2, so `keyjutsu.exe` from the portable build works there.
- **You need someone's help.** `keyjutsu diagnostics preview` prints what they
  would need to know about this machine and KeyJutsu, without your tasks,
  commands, name or secrets; `keyjutsu diagnostics save FILE` writes it out
  for you to read and send. [Privacy](../../PRIVACY.md#diagnostic-bundles)
  lists what's in it.

Next: [your first performance](first-performance.md).
