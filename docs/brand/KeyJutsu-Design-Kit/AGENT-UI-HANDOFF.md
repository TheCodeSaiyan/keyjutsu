# KeyJutsu UI Agent Handoff

Use `KEYJUTSU-DESIGN-SYSTEM.md` and `design-tokens.json` as authoritative UI guidance.

## Hard rules

- Do not invent a second colour system.
- Do not turn KeyJutsu into a generic VS Code clone.
- Do not make agent chat the dominant visual surface.
- Do not use brand vermilion as generic error red.
- Do not force KeyJutsu branding into Performance Terminal Mode.
- Do not use ninja/weapon imagery in functional UI.
- Do not represent `READY` as equivalent to zero risk.
- Do not communicate state by colour alone.

## Implementation preferences

- React + TypeScript UI, Tauri shell.
- Segoe UI Variable for interface typography.
- Cascadia Mono for code/terminal-adjacent content.
- Fluent System Icons for UI iconography.
- Prefer CSS variables generated from the canonical tokens.
- Keep components accessible and keyboard-first.
- Default to dense desktop layouts rather than oversized mobile cards.

## Golden references

Open the four files under `mockups/`. Match their hierarchy and tone before adding new visual patterns.
