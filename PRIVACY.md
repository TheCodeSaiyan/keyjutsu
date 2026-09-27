# Privacy

## What KeyJutsu sends anywhere

KeyJutsu has no telemetry, no crash reporting and no remote diagnostics.
There's no code for any of it, so there's no setting to turn off. It never
looks for updates by itself either.

It makes network requests in two places, and only when you ask:

- When you stage a plan's artifacts (`keyjutsu plan stage`, or Stage in the
  app), it downloads the URLs the plan names, so they can be checked against
  their pinned hashes before anything runs. Nothing is downloaded while a
  plan runs.
- When you check for an update (`keyjutsu update`, or Check for updates in
  the app), it asks GitHub for the list of KeyJutsu's releases, which tells
  GitHub that someone asked, from your address. Installing one downloads
  the installer and its checksums from the release.

AI agents are separate programs you installed and signed in to yourself.
When you ask one for a plan, KeyJutsu runs it on your machine, and the agent
talks to its own provider under your account; what it sends is governed by
that provider, not by KeyJutsu. What KeyJutsu gives the agent is below.

If telemetry is ever considered, it will be off by default and built so
it *cannot* carry prompts, commands, terminal output, file contents or
secrets, by having no path from those to the telemetry code, rather than by
filtering them out on the way.

## What KeyJutsu stores

All of it on this machine, under `%LOCALAPPDATA%\KeyJutsu`:

- **`store`**: the encrypted history of plan runs (the approved plan, its
  checkpoint, the outcome and the Git changes it made), saved Techniques,
  and a record of each plan you approved and each checkpoint KeyJutsu wrote,
  so an edited or copied file is refused. Encrypted with AES-256-GCM under a
  key that only your Windows account on this machine can unlock (DPAPI).
  `keyjutsu store clear --history --techniques` removes those; the
  approval and checkpoint records, which hold only hashes, stay so that
  snapshots you approved still run. `keyjutsu run --ephemeral` records
  nothing in the history.
- **`runs`**: plans approved in the app, as readable JSON, with their
  checkpoints.
- **`artifacts`**: staged downloads and where each came from.
- **`diagnostics`**: diagnostic bundles saved from the app, as plain text.
  They exist only when you save one; see below.
- **Recovery captures**, beside a run's checkpoint: copies of what a
  reversible step said it would change, taken just before it ran, so it can
  be put back.

One thing is kept elsewhere. Before an Administrator step that declares
what it will change, the elevation broker captures that state itself and
keeps it in `%ProgramData%\KeyJutsu\captures`, which only Administrators can
read or write, so it can be put back after a failure. Each capture is
removed once it has been restored, after 30 days, or when KeyJutsu is
uninstalled. It is not encrypted: it holds what the step declared (a file,
a registry value, a service's state) and nothing of your history.

Credentials are never stored: you type them into PowerShell's own masked
prompt, and KeyJutsu doesn't keep them, log them or write them anywhere. The desktop app also
remembers that you have seen the first-run screen, in the webview's local
storage. The installer adds the install folder to your PATH and "Open
KeyJutsu here" to Explorer, if you agree; uninstalling removes both and
leaves the store, as your data.

## Diagnostic bundles

A diagnostic bundle is for you to give someone helping with a problem. It
is built only when you ask, shown to you in full, and saved as plain text;
KeyJutsu never sends it. The app saves only the bundle you previewed.

It holds the KeyJutsu and Windows versions, the readiness checks, the path
and version of each shell and how it behaved when started in a
pseudo-console, your Windows Terminal profile's name, font and command
line, the paths of your PowerShell profile scripts with the three yes/no
findings above, each agent's version and whether its credential file
exists, and how many runs and Techniques the history holds.

It leaves out tasks, plans, commands, terminal contents, what runs printed,
environment variables and credentials. Your profile folder becomes
`%USERPROFILE%`, your Windows user name `<user>` and the computer's name
`<computer>`, and the text goes through the same pattern redaction as what
agents are given. Redaction by pattern can miss a secret with no
recognisable shape, which is one reason the bundle is shown to you first.

## What it reads

- **Windows Terminal settings**, to match your font, colours, cursor and
  padding. Read only, never written.
- **Your PowerShell profile scripts**, scanned for oh-my-posh, Starship and
  PSReadLine settings so KeyJutsu can warn when they may interfere. Only
  those three yes/no findings are kept; the content is not stored or shown.
- **The Windows version**, from the registry, and the versions of the shells
  and tools a plan depends on, to tell whether the machine has changed since
  a plan was approved.
- **Whether each agent is signed in**, by whether its credential file exists.
  The file is never opened.

## Your shell history

With **your own profile**, commands typed in a performance are saved to your
PowerShell history as if you had typed them, and PSReadLine's predictions may
show earlier history entries as grey text while a command is typed, including
during a performance people are watching. With the **clean profile**, KeyJutsu
turns predictions off and saves no history. See
[ADR 0009](docs/architecture/adr/0009-clean-profile-hides-history.md).

## AI agents

Before anything is sent, you see what will go: the task, any text you paste
or files you add, and the folder the agent may investigate. Revisions and
reviews also send the current plan, your guidance and what validation found.
Everything KeyJutsu sends is first redacted of anything that looks like a
secret (tokens, keys, private keys, passwords in assignments). Secrets that
do not look like one are not caught, and a folder you let an agent
investigate is read by the agent itself, not filtered by KeyJutsu; the
preview warns about files there that look sensitive.
