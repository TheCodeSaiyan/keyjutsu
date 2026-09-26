# Colour and design

The canonical design is the official kit in
[`KeyJutsu-Design-Kit/`](KeyJutsu-Design-Kit/README.md):

- [`KEYJUTSU-DESIGN-SYSTEM.md`](KeyJutsu-Design-Kit/KEYJUTSU-DESIGN-SYSTEM.md),
  the design system in words;
- `design-tokens.json`, the machine-readable source of truth;
- [`AGENT-UI-HANDOFF.md`](KeyJutsu-Design-Kit/AGENT-UI-HANDOFF.md), the hard
  rules for anyone, person or agent, building UI;
- `mockups/`, the four golden references.

There is deliberately no second copy of the palette here. The desktop app's
`apps/desktop/src/tokens.css` is generated from `design-tokens.json` by
`pnpm tokens`, and CI fails if it drifts from the kit (`pnpm tokens:check`).
`apps/desktop/src/styles.css` uses those variables and adds no colour, size or
duration of its own.

## Contrast, as measured

The kit asks for "contrast suitable for normal text and statuses". These
are the WCAG 2.1 ratios of the kit's pairs, measured when the palette was first
applied (AA asks 4.5:1 for normal text, 3:1 for large text and for icons):

| Pair | Ratio | Verdict |
| --- | --- | --- |
| Secondary text `#A6A8AC` on Surface 1 `#0F1317` | 7.83 | passes |
| Secondary text `#555A61` on light Surface 1 `#F7F7F5` | 6.48 | passes |
| White `#F7F7F5` on light accent `#D51E22` | 4.85 | passes |
| Error `#D84A4A` on Obsidian `#090B0D` | 4.69 | passes |
| White `#F7F7F5` on vermilion `#E32226` | 4.33 | **fails for normal text** |
| Vermilion focus ring on Obsidian | 4.25 | passes (non-text) |
| Muted text `#747980` on Surface 1 `#0F1317` | 4.25 | **fails for normal text** |
| Error `#D84A4A` on Paper `#FDFCF9` | 4.10 | **fails for normal text** |
| Success `#3FA66B` on `#F7F7F5` | 2.85 | **fails even for icons** |
| Warning `#D89A31` on `#F7F7F5` | 2.28 | **fails even for icons** |

What the app does about it today:

- ARM KEYJUTSU in the dark theme is white on `#E32226`. Its label is 14px
  semibold, which does not meet AA. Either a larger label or the pressed shade
  `#BE1C20` at rest would fix it; that is a decision for the kit's owner, so
  the kit's value is used unchanged.
- Muted text is only used for non-essential detail.
- On the light theme, success and warning glyphs always sit beside a text
  label, so the label carries the meaning.

These are findings to feed back into the kit, not local overrides.
