# Agent integrations

KeyJutsu works with the AI coding agents you already have installed. It runs
each agent's own CLI, never a model API directly (§5), and keeps every one of
them to investigating: an agent proposes, KeyJutsu validates, you approve, and
only KeyJutsu executes (§2.1).

## Compatibility matrix

Every flag below was read from the agent's own `--help` on a real machine.
`keyjutsu agents` shows what is installed and flags any agent whose version
differs from the one its adapter was checked against.

| Agent | Checked against | Run as | Read-only mode | Answer read from |
| --- | --- | --- | --- | --- |
| Codex CLI | 0.154.0 | `codex exec … -` (prompt on stdin) | `--sandbox read-only` | `-o FILE`, the final message |
| Claude Code | 2.1.282 | `claude -p` (prompt on stdin) | `--permission-mode plan` | `--output-format json` envelope |
| Gemini CLI | 0.32.1 | `gemini -p … ` (prompt on stdin) | `--approval-mode plan` | `-o json` envelope |
| GitHub Copilot CLI | 1.0.78 | `copilot -p PROMPT` | `--mode plan --no-ask-user` | `-s`, the response as text |
| Cursor CLI | **not verified** | `cursor-agent -p PROMPT` | unknown | `--output-format json` (unverified) |

No invocation ever passes an agent's bypass or auto-approve options; a test
(`no_invocation_ever_bypasses_the_agents_own_safeguards`) fails if one does.

**Limits, said plainly:**

- **None of the adapters has been exercised against a live agent yet.** They
  were built against the CLIs' documented output shapes and tested with
  recorded answers. `keyjutsu agents check --live` sends each installed agent
  one small request to confirm its adapter works; it uses your accounts, so
  it only runs when asked.
- **Cursor's agent CLI was not installed** where the adapter was written, so
  its flags are unverified; `keyjutsu agents` says so.
- **Copilot takes its prompt on the command line**, which Windows caps at
  32,767 characters. Its prompts use a compact description of the plan format
  instead of the full JSON Schema, and a prompt over 24,000 characters is
  refused rather than truncated.

## What an agent is given

`keyjutsu plan propose` prints the context manifest and sends nothing unless
you add `--send`:

```text
Request for Claude Code 2.1.282, in its read-only mode:
  --permission-mode plan: plan mode, no edits or commands
  file     C:/logs/docker.log (46 characters)
           redacted: GitHub token ×1
  folder   D:/src/app (the agent investigates it itself; KeyJutsu cannot redact what it reads)
           ! looks sensitive: .env

Nothing was sent. Add --send to send this request.
```

- **Files and pasted text** are read by KeyJutsu, redacted by pattern (private
  keys, cloud and platform tokens, `password=`-style assignments), capped at
  16,000 characters and placed in the prompt. The manifest counts what was
  redacted and never shows it.
- **A folder** is where the agent is started. It reads what it likes there,
  with its own tools, so KeyJutsu cannot redact it; the manifest names files
  that look like they hold secrets so you can decide before sending.

## What comes back

Everything an agent returns is untrusted:

- It must parse as a plan proposal: schema, structure, no KeyJutsu-owned
  section. A failing answer is sent back with the problems, twice at most.
- Its `agent` field is overwritten with the agent KeyJutsu actually ran, so an
  agent cannot claim to be another.
- A revision of one step may change that step and nothing else, and it
  discards earlier validation results: the plan has to be validated again.
- A review changes nothing. Its findings are shown to you and recorded in the
  plan's provenance as the reviewer challenging those steps.

## Provenance

Each plan records who did what, in the KeyJutsu-owned part of the plan:

```text
Codex authored wsl-path
Claude challenged wsl-path    (WeakRollback: no way back if the restart fails)
Codex revised wsl-path        (guidance: keep unrelated daemon.json settings)
KeyJutsu validated
```

The approval and the seal follow, in the snapshot itself.
