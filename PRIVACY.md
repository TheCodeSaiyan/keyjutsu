# Privacy

## What KeyJutsu sends anywhere

Nothing. It has no telemetry, no crash reporting, no remote diagnostics, no
update check and no network code. That is true by construction today, not by
a setting: there is no code that could send anything.

When telemetry is considered later (§45), it will be off by default and built
so it *cannot* carry prompts, commands, terminal output, file contents or
secrets, by having no path from those to the telemetry code, rather than by
filtering them out on the way.

## What KeyJutsu stores

Almost nothing, for now:

- The desktop app remembers that you have seen the first-run screen, in the
  webview's local storage.
- Nothing else is persisted. Sessions, plans and history arrive with
  encrypted storage in Milestone 14, protected with a DPAPI-wrapped key.

## What it reads

- **Windows Terminal settings**, to match your font, colours, cursor and
  padding. Read only, never written.
- **Your PowerShell profile scripts**, scanned for oh-my-posh, Starship and
  PSReadLine settings so KeyJutsu can warn when they may interfere. Only
  those three yes/no findings are kept; the content is not stored or shown.
- **The Windows version**, from the registry.

## Your shell history

With **your own profile**, commands typed in a performance are saved to your
PowerShell history as if you had typed them, and PSReadLine's predictions may
show earlier history entries as grey text while a command is typed, including
during a performance people are watching. With the **clean profile**, KeyJutsu
turns predictions off and saves no history. See
[ADR 0009](docs/architecture/adr/0009-clean-profile-hides-history.md).

## AI agents

Not yet integrated. When they are (Milestone 6), you will see exactly what
context goes to the agent before it is sent, and secrets never go (§9, §2.5).
