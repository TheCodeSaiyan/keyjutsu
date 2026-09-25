# KeyJutsu Design System

**Version:** 1.0  
**Status:** Canonical V1 design direction  
**Audience:** Product, frontend, desktop/Tauri, docs, contributors, coding agents

## 1. Design thesis

KeyJutsu should look like a serious Windows engineering tool that happens to have a clever premise. The planning workspace is recognisably KeyJutsu; armed terminal mode deliberately is not.

The visual language combines:

- Windows-native interaction conventions;
- Japanese minimalism through restraint, spacing, geometry and vermilion accents;
- dense but legible technical information;
- strong semantic state communication;
- almost no decorative chrome.

**Do not turn the product into cyberpunk UI, a ninja-themed novelty app, or a VS Code clone.**

The brand ratio should stay approximately **70–80% neutral surfaces, 15–25% secondary neutrals, under 5% vermilion**.

---

## 2. Brand assets

Canonical assets live under `assets/brand/`.

Primary roles:

| Asset | Intended use |
|---|---|
| `keyjutsu_keycap_logo.png` | Primary horizontal brand lockup |
| `keyjutsu_wordmark_on_transparency.png` | Wordmark-only placements |
| `keyjutsu_monogram_logo.png` | Alternate lockup / narrow banners |
| `glossy_keyjutsu_emblem_icon.png` | Primary application icon master |
| `keycap_monogram_app_icon.png` | Light-background application icon option |
| `monochrome_geometric_keyboard_icon.png` | Single-colour/fallback contexts |
| `monochrome_keyjutsu_keycap_logo.png` | Monochrome horizontal lockup |
| `keyjutsu_terminal_banner.png` | Marketing/social banner artwork |
| `keyjutsu_terminal_mastery.png` | Alternate branded banner artwork |

### Clear space

Keep at least **one red-slash width** of clear space around the emblem or lockup. Do not crop the keycap silhouette tightly enough that the lower corner geometry touches surrounding content.

### Minimum sizes

- App emblem: 16 px minimum only for system icon contexts; prefer 20/24/32 px in UI.
- Horizontal lockup: 140 px wide minimum in normal screens.
- Never use the full lockup inside dense table rows or toolbar buttons; use the emblem.

### Never

- recolour the vermilion accent to semantic green/yellow;
- add neon glow as standard treatment;
- stretch the logo;
- place text over the emblem;
- put the full brand lockup inside armed terminal mode.

---

## 3. Colour

The authoritative values are in `design-tokens.json` and `design-tokens.css`.

### Brand

| Token | Hex | Purpose |
|---|---:|---|
| Obsidian | `#090B0D` | Deep application background |
| KeyJutsu Black | `#0F1317` | Primary dark structural colour |
| Graphite | `#1A1E23` | Panels and raised surfaces |
| Gunmetal | `#35373E` | Borders and structural contrast |
| Steel | `#62666D` | Muted controls / inactive UI |
| Mist | `#A6A8AC` | Secondary foreground |
| KeyJutsu White | `#F7F7F5` | Primary light foreground |
| Paper | `#FDFCF9` | Warm light-theme background |
| Vermilion | `#E32226` | Brand/interaction accent |

### Semantic state

| State | Colour |
|---|---:|
| Ready / Success | `#3FA66B` |
| Information / Running | `#4C8DDB` |
| Review / Warning | `#D89A31` |
| Error / Blocked | `#D84A4A` |
| Critical | `#B72C3B` |
| Waiting for user | `#AF78D2` |
| Performance typing | `#E32226` |

Brand red is **not** the generic error colour. This distinction is required.

No state may be communicated through colour alone. Pair colour with an icon and text label.

---

## 4. Typography

### UI family

```css
font-family: 'Segoe UI Variable', 'Segoe UI', system-ui, -apple-system, sans-serif;
```

### Monospace family

```css
font-family: 'Cascadia Mono', 'Cascadia Code', 'Consolas', monospace;
```

### Scale

| Role | Size / line | Weight |
|---|---|---|
| Display | 32 / 40 | 700 |
| H1 | 24 / 32 | 600–700 |
| H2 | 20 / 28 | 600 |
| H3 / panel heading | 16 / 24 | 600 |
| Body large | 16 / 24 | 400 |
| Body | 14 / 20 | 400 |
| Compact | 12 / 16 | 400–500 |
| Caption | 11 / 16 | 400–500 |
| Code | 13 / 20 | 400 |

Use uppercase only for very short structural labels such as `READY`, `CRITICAL`, or `PHASE 2`. Use 0.08em tracking for those labels.

---

## 5. Spacing, radii and density

Use a **4 px base grid**. Preferred increments: 4, 8, 12, 16, 20, 24, 32, 40, 48, 64.

### Radius

- tiny inline chips: 6 px
- controls/cards: 8 px
- dialogs / major surfaces: 12 px
- large onboarding surfaces: 16 px

Avoid excessively rounded “mobile app” cards.

### Density

KeyJutsu defaults to a technical desktop density:

- standard control height: 36 px
- large primary action: 44 px
- plan row: 40 px
- compact plan row: 32 px
- standard panel padding: 16 px
- page/workspace outer padding: 24 px

---

## 6. Surface hierarchy

Prefer **subtle tonal separation and one-pixel borders** over nested card stacks.

Dark hierarchy:

1. `#090B0D` — application background
2. `#0F1317` — primary shell/navigation
3. `#1A1E23` — working panel
4. `#23282E` — nested/selected region
5. `#2C3138` — hover/raised state

Do not wrap every section in its own rounded card. Dense technical workspaces should feel continuous.

---

## 7. Buttons

### Primary

Use vermilion only for deliberate high-value action. Examples:

- `ARM KEYJUTSU`
- confirm a reviewed plan
- resume approved execution

Do not make every `Save`, `Next`, or `Close` button red.

### Secondary

Graphite/neutral surface with border.

### Destructive

Use semantic critical/error treatment, **not brand vermilion**.

### Text buttons

Use for low-priority actions such as `Dismiss`, `View details`, `Copy command`.

---

## 8. Form controls

Controls should feel Windows-native rather than web-form generic.

- 36 px default height
- 8 px radius
- one-pixel border
- clear keyboard focus ring
- selected/focused accent may use vermilion
- validation errors use semantic error red
- credential fields never expose actual values to generic UI logging

---

## 9. Navigation

Use a compact left rail/sidebar for major spaces:

- New Task
- Sessions
- Techniques
- Agents
- Settings

The main workflow should not require deep navigation. A task stays in one adaptive workspace as it evolves from prompt → plan → validation → execution.

Use icons plus labels in expanded state; icons only in collapsed state with accessible tooltips.

---

## 10. Plan row anatomy

A plan step row should expose, without opening details:

1. ordinal/graph indicator;
2. step title;
3. shell/runtime hint if relevant;
4. state (`READY`, `REVIEW`, `BLOCKED`, `RUNNING`, etc.);
5. risk indicator only when materially relevant;
6. optional privilege/elevation glyph;
7. expand/inspect affordance.

Do not show raw command bodies in every collapsed row.

### Selected row

Use a low-opacity vermilion tint or stronger neutral border, not a solid red fill.

---

## 11. State language

Canonical labels:

- `READY`
- `REVIEW`
- `BLOCKED`
- `RUNNING`
- `VALIDATING`
- `WAITING FOR USER`
- `REVALIDATION REQUIRED`
- `FAILED`
- `SKIPPED`
- `COMPLETE`

State labels must be stable across desktop, CLI and documentation.

---

## 12. Risk language

Risk is separate from readiness.

A step can be both `READY` and `HIGH` risk.

Canonical risk levels:

- Low
- Normal
- High
- Critical

Never conflate “validated” with “safe”.

---

## 13. Critical-action visual language

Critical actions must look materially different from normal warnings.

Use:

- semantic critical red `#B72C3B`;
- explicit `CRITICAL ACTION` heading;
- danger icon + text;
- exact expanded target;
- impact;
- reversibility;
- recovery availability;
- contextual typed confirmation.

Do not use animation, flashing, countdowns or panic language.

---

## 14. Credential visual language

Credential gates should feel secure and calm.

- subtle lock/shield icon;
- purple `WAITING FOR USER` state may identify genuine input handoff;
- fake typing must visibly/semantically be disabled;
- do not display credential values in session summaries;
- never imply credentials were “AI generated”.

---

## 15. Agent identity

Agent branding is secondary to KeyJutsu.

Show:

- agent name;
- compact provider icon if licensing permits;
- CLI version/readiness;
- primary vs reviewer role.

Do not recolour the whole workspace to match agent brands.

Example:

`Codex CLI · Primary · Ready`

`Claude Code · Reviewer`

---

## 16. Plan graph grammar

Use restrained technical graph styling.

### Nodes

- normal: neutral border
- selected: vermilion-accented border/tint
- ready: success state chip
- review: warning state chip
- failed: error state chip
- skipped branch: reduced opacity + dashed edge

### Edges

- standard dependency: 1 px muted line
- active execution path: info/brand accent depending context
- conditional branch: labelled edge (`WSL2`, `Hyper-V`, etc.)
- invalid/cyclic dependency: error treatment

Keep graphs readable; do not turn them into decorative node maps.

---

## 17. Technique cards

A Technique summary should show:

- name;
- one-line purpose;
- version/revision;
- target compatibility;
- last validated timestamp;
- current drift status;
- parameter count;
- run/revalidate action.

Example state:

`REVALIDATION REQUIRED — Docker version changed`

---

## 18. Terminal and armed mode

This is intentionally different from the rest of the product.

When armed:

- remove KeyJutsu branding;
- use the selected/preflighted real terminal profile;
- preserve the user's font, colour scheme and prompt where compatible;
- do not force KeyJutsu's dark palette on the terminal;
- hide plan chrome;
- allow a discreet operator overlay only via configured shortcut;
- retain hard-disarm and genuine interrupt controls.

The terminal is real. The typing is theatre.

---

## 19. Operator overlay

The overlay should be small, private-looking and non-disruptive.

Display only what the operator needs:

- current step;
- current state;
- next step title;
- mode (`Performance`, `Auto Performance`, etc.);
- `Pause`, `Disarm`, `Open plan`;
- a subtle progress indicator.

Do not cover large portions of terminal output.

---

## 20. Motion

Motion should communicate state changes rather than entertain.

Recommended durations:

- micro interaction: 80–120 ms
- ordinary transition: 180 ms
- panel open/close: 240 ms
- complex state transition: max 320 ms

Use `cubic-bezier(.2,0,0,1)` as default.

No bouncing. No glowing pulses. No looping status animation except subtle progress indicators where the process truly remains active.

Respect `prefers-reduced-motion`.

---

## 21. Iconography

Preferred system: **Fluent System Icons**.

Use Regular outline icons by default and Filled variants only for selected/armed/emphasised states.

Core semantic icon concepts:

| Concept | Icon meaning |
|---|---|
| Task | prompt/document |
| Plan | list/branch |
| Validate | check/shield-check |
| Ready | check circle |
| Review | warning/eye |
| Blocked | error circle |
| Credentials | key/lock |
| Elevation | shield |
| Network | globe/plug |
| Git | branch |
| Recovery | arrow undo/history |
| Agent review | people/chat/check |
| Arm | play/shield or bespoke KeyJutsu action mark |

Do not use swords, shuriken, masks or ninja silhouettes as functional icons.

---

## 22. Brand voice

### Voice

- concise
- technical
- calm
- transparent about uncertainty
- lightly playful only where it does not reduce clarity

### Good

- `2 steps require revalidation.`
- `Docker 30.1 differs from the version this Technique was last validated against.`
- `Ready to arm.`
- `This action cannot be rolled back.`

### Avoid

- `AI magic is ready!`
- `Hacker mode activated!`
- `This is 100% safe.`
- `Ninja skills engaged.`

The product name supplies enough personality by itself.

---

## 23. Empty, loading and error states

Every state should tell the user what matters next.

### Empty

`No Techniques yet. Successful sessions can be promoted into reusable Techniques.`

### Planning

`Codex is investigating the selected context…`

### Blocked

`This step cannot become Ready until PowerShell 7.6 or compatible syntax is available.`

### Failure

Show expected vs actual, then actionable choices. Do not use generic “Something went wrong” when structured error information exists.

---

## 24. Accessibility

Mandatory:

- native/semantic controls;
- full keyboard workflow;
- visible focus state;
- screen-reader labels;
- 44 px targets for important touch/click actions where practical;
- contrast suitable for normal text and statuses;
- no colour-only semantics;
- scalable text/layout;
- reduced-motion mode.

---

## 25. Asset matrix

### Windows/Tauri app icon

Provide these PNG sizes:

- 16
- 20
- 24
- 32
- 40
- 48
- 64
- 96
- 128
- 256
- 512
- 1024

Also provide multi-resolution `keyjutsu.ico` containing at least 16, 20, 24, 32, 40, 48, 64, 128 and 256 px.

Generated versions are included in `assets/icons/`.

### GitHub / docs

- repository avatar: 512 × 512 emblem
- social preview: 1280 × 640 or equivalent wide banner
- README horizontal mark: transparent PNG, 1600+ px preferred
- favicon: 32 × 32 and ICO

### Installer

Use the application emblem, not the full wordmark, for installer/system iconography.

---

## 26. Do / Don't

### Do

- use neutral surfaces and thin borders;
- let vermilion identify intentional interaction;
- make the plan and current state obvious;
- use compact technical layouts;
- preserve whitespace around high-risk actions;
- keep the actual terminal authentic;
- make uncertainty explicit.

### Don't

- make every panel a floating card;
- use red everywhere;
- turn warnings into dramatic theatre;
- use cyberpunk green/blue gradients;
- mix icon families;
- hide risk behind hover states;
- let agent chat visually dominate the execution plan;
- leave mock terminal output disconnected from the real PTY.

---

## 27. Canonical mockups

Reference mockups are under `mockups/`:

1. `01-new-task.html` / `.png` — initial task intake
2. `02-plan-workspace.html` / `.png` — planning, validation and approval
3. `03-critical-action.html` / `.png` — semantic confirmation gate
4. `04-armed-terminal-overlay.html` / `.png` — real-terminal presentation with operator overlay

These are **directional golden references**, not pixel-perfect implementation contracts. The implementation should preserve their hierarchy, density, tone, and state semantics while respecting real Windows/Tauri constraints.

---

## 28. Agent implementation rule

When implementing KeyJutsu UI, treat this design system and `design-tokens.json` as the visual source of truth.

If a component requires a visual decision not covered here:

1. choose the least decorative option;
2. preserve semantic clarity;
3. prefer Windows-native behaviour;
4. reuse existing tokens;
5. document any new token rather than inventing one-off values throughout the codebase.
