# 0013: The design kit is canonical; the app's tokens are generated from it

Status: accepted, 25 September 2026.

## Context

The official KeyJutsu design kit arrived in `docs/brand/KeyJutsu-Design-Kit`:
a design system, `design-tokens.json`, an agent handoff and four mockups. Its
first hard rule is "do not invent a second colour system". The desktop app
already had a hand-written palette in `styles.css`, taken from an earlier
colour brief, with the same colours but its own variable names.

## Decision

- The kit is the source of truth for visual design. Nothing in the repository
  restates its palette; `docs/brand/colour-system.md` points at it and keeps
  only the contrast measurements the kit does not include.
- `apps/desktop/src/tokens.css` is generated from `design-tokens.json` by
  `scripts/design-tokens.mjs`, using the kit's own variable names. CI runs
  `pnpm tokens:check` and fails if the file is stale.
- `styles.css` uses those variables and adds no colour, size, radius or
  duration of its own (§28: "reuse existing tokens").
- The app icon comes from the kit's icon set, including its multi-size
  `keyjutsu.ico` (16 to 256 px), rather than being generated from one master.
- The README uses the kit's lockups directly.

## What was applied now, and what waits

Applied to the screens that exist: type scale, 4px spacing grid, 8px control
radius, 36px controls and a 44px ARM button, the vermilion focus ring, hairline
surfaces, and an operator overlay with the contents §19 lists (mode, current
and next step, state, progress, Resume and Disarm).

Waiting for the milestones that build the screens they describe: the left rail
(New Task, Sessions, Techniques, Agents, Settings), task intake (mockup 01), the
plan workspace (02) and the critical-action gate (03). Adding a rail now would
mean navigation to places that do nothing.

Fluent System Icons, the kit's preferred icon set, are not added yet: the
current screens use text glyphs, and adding the icon package is part of
building the rail.

## Consequences

The kit's mockups are "directional golden references, not pixel-perfect
contracts", and their PNG renders have overlapping controls; where a PNG and
the design system disagree, the design system wins.

The contrast findings in `colour-system.md` (white on vermilion is 4.33:1, short
of AA for normal text) are reported back rather than patched locally, because a
local fix would be exactly the second colour system the kit forbids.
