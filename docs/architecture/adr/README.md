# Architecture decision records

One decision per file: the context, what was decided, and what it costs. A
superseded record is kept and marked, never rewritten, so the history of why
the code looks the way it does stays readable.

| ADR | Decision | Status |
| --- | --- | --- |
| [0001](0001-rust-decides-react-asks.md) | The Rust core is the authority; the React front end only asks | Accepted |
| [0002](0002-conpty-through-portable-pty.md) | ConPTY through `portable-pty` | Accepted |
| [0003](0003-completion-from-prompt-marks.md) | Command completion from nonce-stamped OSC 133 prompt marks | Accepted |
| [0004](0004-pure-performance-engine.md) | The performance engine is a pure state machine | Accepted |
| [0005](0005-crates-arrive-with-consumers.md) | Crates are created when something uses them | Accepted |
| [0006](0006-disarm-chord-and-escape.md) | The disarm chord is Ctrl+Alt+Shift+K and Esc keeps its meaning | Accepted |
| [0007](0007-typescript-generated-from-rust.md) | TypeScript IPC types are generated from Rust | Accepted |
| [0008](0008-restore-ctrl-c-for-shells.md) | Clear the inherited Ctrl+C-ignore flag before starting shells | Accepted |
| [0009](0009-clean-profile-hides-history.md) | The clean profile turns off history predictions and saving | Accepted |
| [0010](0010-plan-hashing.md) | Canonical JSON and SHA-256 for plan and step hashes | Accepted |
| [0011](0011-elevation-broker.md) | A separate, narrowly scoped elevation broker | Accepted |
| [0012](0012-gpl-3-licence.md) | KeyJutsu is licensed GPL-3.0-only | Accepted |
| [0013](0013-design-kit-is-canonical.md) | The design kit is canonical; the app's tokens are generated from it | Accepted |
| [0014](0014-validation-runs-nothing-it-validates.md) | Validation runs nothing a plan names, with one guarded exception | Accepted |
| [0015](0015-credentials-through-the-shells-masked-prompt.md) | Credentials are typed into the shell's own masked prompt | Accepted |
| [0016](0016-encrypted-file-store.md) | History is an encrypted file store under a DPAPI-protected key | Accepted |
| [0017](0017-broker-keeps-administrator-captures.md) | The broker captures and restores what an Administrator step changes | Accepted |
| [0018](0018-file-contents-kept-encrypted.md) | Copies of files are kept encrypted; plans and snapshots stay readable | Proposed |
