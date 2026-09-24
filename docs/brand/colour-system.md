# KeyJutsu colour system

The canonical colours. The KeyJutsu logo is the source of truth for them, and
`apps/desktop/src/styles.css` implements them as CSS tokens. If the two
disagree, this page wins and the stylesheet needs correcting.

## Design intent

KeyJutsu uses a restrained Japanese-minimal, technical Windows aesthetic:

- near-black graphite surfaces
- warm off-white foregrounds
- neutral steel greys
- a single strong vermilion brand accent, which should feel deliberate, not
  omnipresent
- no neon or cyberpunk colours
- no large red background areas
- subtle borders and tonal surface differences rather than heavy shadows

The UI should feel premium, technical and calm.

## Core brand palette

| Name | Hex | RGB | Use |
| --- | --- | --- | --- |
| KeyJutsu Black | `#0F1317` | 15, 19, 23 | Dark foregrounds, dark logo elements, primary dark surfaces, title bars, strong text on light backgrounds |
| Obsidian | `#090B0D` | 9, 11, 13 | Deepest background: dark-mode root, terminal-adjacent surfaces, high-contrast framing. Do not use pure `#000000` everywhere. |
| Graphite | `#1A1E23` | 26, 30, 35 | Primary elevated surface: panels, cards, navigation, dialogs, plan workspace |
| Gunmetal | `#35373E` | 53, 55, 62 | Borders, separators, disabled surfaces, secondary controls |
| Steel | `#62666D` | 98, 102, 109 | Secondary text, metadata, inactive icons, subtle outlines |
| Mist | `#A6A8AC` | 166, 168, 172 | Sparingly: secondary text on dark, placeholders, non-critical metadata |
| KeyJutsu White | `#F7F7F5` | 247, 247, 245 | Main text on dark surfaces, light logo areas, light-theme surfaces. Slightly warm, not pure white. |
| Paper | `#FDFCF9` | 253, 252, 249 | Light-theme background: paper-like warmth without texture |

## Brand accent

| Name | Hex | RGB |
| --- | --- | --- |
| KeyJutsu Vermilion | `#E32226` | 227, 34, 38 |
| Vermilion Hover | `#F0393D` | 240, 57, 61 |
| Vermilion Pressed | `#BE1C20` | 190, 28, 32 |
| Vermilion Tint | `#E3222618` | translucent |

Vermilion is the **primary brand accent**: primary action highlight,
selected or focused state, the logo, ARM KEYJUTSU, important active controls,
thin accent strokes, and progress or current-step indication. The tint suits
selected rows, chips and focus regions.

It is **not** the error colour. Brand red and error red must stay
semantically distinguishable.

## Dark theme

| Token | Hex |
| --- | --- |
| Background | `#090B0D` |
| Surface 1 | `#0F1317` |
| Surface 2 | `#1A1E23` |
| Surface 3 | `#23282E` |
| Raised / hover surface | `#2C3138` |
| Border | `#353A41` |
| Border subtle | `#252A30` |
| Primary text | `#F7F7F5` |
| Secondary text | `#A6A8AC` |
| Muted text | `#747980` |
| Disabled text | `#555A61` |
| Accent | `#E32226` |

## Light theme

| Token | Hex |
| --- | --- |
| Background | `#FDFCF9` |
| Surface 1 | `#F7F7F5` |
| Surface 2 | `#EEEDEA` |
| Surface 3 | `#E5E4E1` |
| Raised / hover surface | `#FFFFFF` |
| Border | `#D4D3D0` |
| Border strong | `#BABAB7` |
| Primary text | `#0F1317` |
| Secondary text | `#555A61` |
| Muted text | `#747980` |
| Disabled text | `#A6A8AC` |
| Accent | `#D51E22` |

## Semantic colours

Functional colours. They never replace the brand red.

| Meaning | Hex | Use |
| --- | --- | --- |
| Ready / success | `#3FA66B` | READY, passed validation, successful execution, healthy state |
| Information | `#4C8DDB` | Informational messages, read-only discoveries, neutral agent information |
| Warning | `#D89A31` | Review required, partial reversibility, environment drift, elevated caution |
| Error | `#D84A4A` | Failed commands, validation failure, runtime error. Keep visually distinct from brand vermilion. |
| Critical | `#B72C3B` | Destructive actions, irreversible actions, critical-action gates. Always with icons, text or borders too; never colour alone. |

## Plan states

| State | Hex |
| --- | --- |
| READY | `#3FA66B` |
| REVIEW | `#D89A31` |
| BLOCKED | `#D84A4A` |
| CRITICAL | `#B72C3B` |
| RUNNING | `#4C8DDB` |
| TYPING / PERFORMANCE ACTIVE | `#E32226` |
| WAITING FOR USER | `#AF78D2` |
| DISABLED / SKIPPED | `#62666D` |

## Usage rules

1. Vermilion is the brand accent, not the default background colour.
2. Aim for roughly 70–80% neutral dark or light surfaces, 15–25% secondary
   neutral tones, and under 5% KeyJutsu red.
3. Reserve brand red for selection, the active or current state, focus, the
   logo, ARM KEYJUTSU, Performance Mode and important intentional actions.
4. READY is green, not red.
5. ERROR and CRITICAL use their semantic reds, not brand red.
6. No gradients as the normal UI treatment. The logo and assets may have
   depth and shading; application surfaces stay clean and restrained.
7. Avoid bright white (`#FFFFFF`) for large dark-theme text areas; prefer
   `#F7F7F5`.
8. Avoid pure black (`#000000`) for large surfaces; prefer `#090B0D` or
   `#0F1317`.
9. Use subtle 1px graphite or steel borders for hierarchy rather than
   wrapping every element in a card.
10. Performance Terminal Mode does not force the KeyJutsu palette. It
    inherits and matches the user's actual terminal profile.
11. KeyJutsu branding belongs to the planning and operator interface. Armed
    into stealth Performance Mode, the branding disappears.
12. Colour is never the only indicator of risk, readiness, failure, approval
    state or critical and destructive actions. Always pair it with text or
    iconography.

## CSS tokens

```css
:root {
  --kj-black: #0f1317;
  --kj-obsidian: #090b0d;
  --kj-graphite: #1a1e23;
  --kj-gunmetal: #35373e;
  --kj-steel: #62666d;
  --kj-mist: #a6a8ac;
  --kj-white: #f7f7f5;
  --kj-paper: #fdfcf9;
  --kj-red: #e32226;
  --kj-red-hover: #f0393d;
  --kj-red-pressed: #be1c20;
  --kj-success: #3fa66b;
  --kj-info: #4c8ddb;
  --kj-warning: #d89a31;
  --kj-error: #d84a4a;
  --kj-critical: #b72c3b;
  --kj-user-input: #af78d2;
}
```

The theme tokens (`--background`, `--surface-1` to `--surface-3`,
`--surface-hover`, `--border`, `--border-subtle`, `--text-primary`,
`--text-secondary`, `--text-muted`, `--text-disabled`, `--accent`) take the
dark and light values in the tables above. In the desktop app they follow the
system theme through `prefers-color-scheme`.

## Contrast notes

Measured against WCAG 2.1 (AA asks 4.5:1 for normal text, 3:1 for large
text and for non-text marks such as icons):

| Pair | Ratio | Verdict |
| --- | --- | --- |
| Secondary text `#A6A8AC` on Surface 1 `#0F1317` | 7.83 | passes |
| Secondary text `#555A61` on Surface 1 light `#F7F7F5` | 6.48 | passes |
| White `#F7F7F5` on light accent `#D51E22` | 4.85 | passes |
| Error `#D84A4A` on Obsidian `#090B0D` | 4.69 | passes |
| White `#F7F7F5` on vermilion `#E32226` | 4.33 | **fails for normal text**; passes only at 18.7px bold or 24px |
| Vermilion focus ring `#E32226` on Obsidian | 4.25 | passes (non-text, 3:1) |
| Muted text `#747980` on Surface 1 `#0F1317` | 4.25 | **fails for normal text** |
| Error `#D84A4A` on Paper `#FDFCF9` | 4.10 | **fails for normal text** |
| Success `#3FA66B` on `#F7F7F5` | 2.85 | **fails even for icons** |
| Warning `#D89A31` on `#F7F7F5` | 2.28 | **fails even for icons** |

Consequences for how the app uses them today:

- ARM KEYJUTSU in the dark theme is white on `#E32226` with a 14px label,
  which does not meet AA. Options: a larger label, or the pressed shade
  `#BE1C20` as the resting button colour. A decision for the owner of this
  palette, not something to change quietly.
- Muted text is kept for non-essential detail; anything a user must read uses
  secondary text.
- On the light theme, status glyphs in success and warning colours sit next
  to their text label, so the label carries the meaning; the glyph colour is
  decoration there. Darker light-theme variants would fix it properly.
