# 0019: Updates come through the signed installer, when the operator asks

Status: proposed, 27 September 2026.

## Context

The specification asks for an updater: signed, under the user's control,
never replacing KeyJutsu in the middle of a task, and checked for protocol
compatibility with the broker, with Stable and Beta channels. KeyJutsu has
none. A new version is a new installer the operator downloads and runs.

Three things constrain how an updater can work here:

- **KeyJutsu makes no network requests of its own.** The only one is
  staging a plan's artifacts, which the operator asks for. `PRIVACY.md` and
  the README say there is no update check. Checking for updates on its own,
  even once a day, would change that promise, and a check says to GitHub
  which version is running, and when.
- **KeyJutsu is installed for every user, in Program Files**, because the
  elevation broker must live where only Administrators can replace it
  ([ADR 0011](0011-elevation-broker.md)). Replacing it needs Administrator,
  whatever does the replacing.
- **The installer and every program in it are signed** with the
  publisher's certificate through Artifact Signing, and `install.ps1`
  already checks a download against `SHA256SUMS` and its Authenticode
  signature before running it.

And one practical fact: while the repository is private, its release files
cannot be downloaded without signing in to GitHub, so no updater, and not
`install.ps1` either, can fetch a release until the repository is public.

## Options

1. **Tauri's updater plugin.** The app downloads an update package signed
   with a key of Tauri's own (minisign) and applies it. One click, but a
   second signing key to create, store and rotate beside the Authenticode
   certificate; an update package format beside the installer; and, for a
   per-machine install, a UAC prompt anyway, since the files are in Program
   Files.
2. **Check when asked, and hand over to the signed installer.** "Check for
   updates" in the app, and a new update command in the CLI, ask GitHub for the newest
   release on the operator's channel, show its version and notes, and on
   the operator's word download the installer, check it exactly as
   `install.ps1` does (`SHA256SUMS`, then its signature, which must be
   KeyJutsu's publisher), and run it. Windows asks for Administrator once,
   as for any install. Nothing new to sign: the installer is the update.
3. **winget.** `winget upgrade TheCodeSaiyan.KeyJutsu` updates like any
   other package. Only once the repository is public and the package is in
   winget-pkgs, and no channel choice.

## Decision (proposed)

Option 2, with winget as well once the repository is public.

- **The operator starts every check.** No check runs on its own, so the
  privacy promise changes only to say that asking for an update asks
  GitHub. A setting to check at start-up could come later, off by default.
- **Never during a run.** The app and CLI refuse to start an update while a
  run or recovery holds the run lock, and the installer's own pre-install
  hook refuses while the lock is held, however it was started, so a manual
  install cannot replace KeyJutsu mid-task either.
- **Signed and checked before anything runs.** The download is checked
  against the release's `SHA256SUMS` and its Authenticode signature, which
  must name KeyJutsu's publisher; anything else is deleted, not run.
- **Channels.** Stable is the newest release; Beta is the newest
  pre-release. The operator chooses; Stable by default.
- **Protocol compatibility.** The broker and the programs that start it are
  installed together by the one installer, and a broker of another protocol
  version is already refused. An update is whole, never partial.

## Consequences

- `PRIVACY.md` and the README change: KeyJutsu makes network requests when
  the operator stages artifacts, or asks whether there is an update.
- Until the repository is public, the check has nothing it can download;
  it says so rather than failing.
- The installer gains a pre-install check of the run lock.
- Tests: the version comparison and channel choice; refusal while the run
  lock is held (app, CLI and installer hook); a download that fails its
  checksum or signature is deleted and not run.
