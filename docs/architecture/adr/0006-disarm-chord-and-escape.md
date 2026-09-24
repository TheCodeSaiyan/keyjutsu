# 0006: The disarm chord is Ctrl+Alt+Shift+K and Esc keeps its meaning

Status: accepted, Milestone 2.

## Decision

- Hard disarm: Ctrl+Alt+Shift+K. Operator overlay: Ctrl+Shift+K. Both
  configurable through `Bindings`, which refuses a chord without Ctrl, Alt or
  Win (ordinary typing could trigger it), a chord that is Ctrl+C, or the same
  chord for both.
- The disarm chord is classified before anything else looks at a key, and the
  engine handles it before checking whether it owns the keyboard, so there is
  no state in which it is consumed as staged typing.
- Letters in chords match regardless of case, because Shift changes what the
  layout reports. The desktop reads a letter chord from the physical key
  (`KeyboardEvent.code`) when Ctrl, Alt or Win is held, since layouts report
  anything from `K` to a control character for that combination; AltGr
  characters such as `@` or `€` are left as typed.
- Esc is passed to a running command and swallowed while KeyJutsu owns the
  input line, never a disarm. See
  [the state machine](../execution-state-machine.md#decisions-worth-knowing)
  for why swallowing is necessary.

## Evidence

The chord works end to end through three different input paths: engine
tests; the CLI test, which sends it to `keyjutsu.exe` as a genuine Win32 key
event through a pseudo-console; and the desktop app, driven with `SendKeys`
while armed, which disarmed and handed the terminal back.

## Limits

The CLI has no overlay to draw, so there the overlay chord pauses and resumes
without showing anything.
