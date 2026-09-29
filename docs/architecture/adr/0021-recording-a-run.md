# 0021: Recording a run, and exporting it as video, GIF or a guide

Status: accepted, 28 September 2026.

## Context

A run that worked is worth showing: to a colleague who has to do the same,
in a ticket, or as the start of a how-to. KeyJutsu keeps what a run did (its
history record and checkpoint) but not what it looked like, and a screen
recorder captures the window, the cursor and whatever else is on the screen,
with no idea where one step ends and the next begins.

KeyJutsu already has what a good recording needs: the exact stream the
terminal draws, byte for byte, and the moment each step starts and finishes.

## Options

1. **Capture the screen.** Faithful to what was seen, but large, tied to the
   window's size and position, blind to steps, and it records anything that
   covers the window.
2. **Capture the terminal's canvas while it runs.** Knows nothing of steps
   either, and costs the performance frames while it encodes.
3. **Record the stream, render on export.** Keep the output with its timings
   and a marker at each step boundary; replay it off-screen when the operator
   exports, in whatever form and whatever cut they choose.

## Decision

Option 3.

- **What is recorded:** the terminal output with the time it arrived, and a
  marker when each plan step starts and finishes, in asciicast v2, the format
  asciinema uses. It is small, text, and replays exactly.
- **When:** only when the operator switches recording on for a run, in the
  app or with `keyjutsu run --record`. It is kept with that run's history
  record, encrypted like it, and goes when the history does.
- **Export**, from the run's panel and from History:
  - **Which part:** the whole run, a range of steps, or each step as its own
    file. A cut that starts part-way replays what came before at once, so the
    first frame is the screen as it was, not a blank one.
  - **As what:** a soundless video (MP4 where the app's browser engine can
    record it, WebM otherwise, decided when exporting), a GIF, an asciicast
    `.cast`, or a step-by-step guide, as a Markdown folder (`guide.md` and one
    image per step) and as a single HTML page with the images inside.
  - Long pauses are shortened (to two seconds), as asciinema does, so a slow
    download does not make a minute of still frames.
  - Video and GIF are made by replaying the recording into a headless
    terminal and painting its cells, in the terminal's own font and colours;
    the CLI exports `.cast` only.
- **Privacy:** what a command prints can include what should not be shared.
  An export is redacted with the same patterns as anything sent to an agent:
  a secret is found in the joined text of a step, so one typed a character at
  a time is found too, and each of its characters is replaced by `*` where it
  fell, which keeps the timing and the layout. The operator sees the first
  frames, and the redaction count, before anything is saved. A secret typed
  at a credential step is never in the recording: PowerShell's masked prompt
  does not echo it. The account's name, and its profile folder's where that
  differs, is masked the same way wherever it stands on its own, in the
  recording and in the guide's text from the plan, since every path under the
  profile names it. This is done at export, not in the store, so a recording
  made before it is covered too.

### Sizes

A terminal's output is drawn for its size: the cursor is placed by row and
column, so output replayed at another size lands in the wrong places. The
first recordings took their size from the window, which had just resized
itself for the run, and replayed garbled. So:

- A recording's size, and the screen it starts from, come from the shell's
  own session, which knows the size it draws for. The screen is captured
  as it is when recording starts, so the first frame is whole.
- Every change of size is recorded (asciicast `r` events) and replayed.
- A cut opens on the screen as it was at that moment, at the size it had.
- An export is fitted to the part of the screen the run drew on, or drawn
  at the most the terminal had, in a chosen text size.

## Consequences

- One new dependency for GIF encoding (`gifenc`), and `@xterm/headless` from
  the terminal library already used.
- A recording is only as long as the run; an export of video plays in real
  time with pauses shortened, so a long run takes a while to export. Windows
  slows a window that is hidden or minimised, and with it the replay, so the
  export asks for the window to stay in view.
- The guide's text comes from the plan (title, objective, reason, commands)
  and the recording (what each step printed), redacted; its pictures are the
  last frame of each step, or a GIF of it.
- Tests: recording and markers; cutting by step, with the earlier output
  replayed at the start; redaction across writes, keeping lengths; the
  asciicast written and read back; the CLI's `--record` and `.cast` export.
