# 0015: Credentials are typed into the shell's own masked prompt

Status: accepted, Milestone 9.

## Context

§2.5 and §25: a secret must be real user input, never staged typing, and must
not appear in the plan, agent context, terminal replay, logs or telemetry. A
plan still has to *use* the secret, for example to sign in to a registry.

Where the secret lives decides where it can leak. If KeyJutsu read it (from a
dialog of its own, or by intercepting keys), KeyJutsu would hold it in memory
and then have to hand it to the shell somehow. Typing it as `$env:X = '…'`
would draw it on screen and put it in PSReadLine's history, and passing it on
a command line would expose it to anything that can list processes.

## Decision

A credential step says what it asks for, never the value:

```json
"credential": {"variable": "REGISTRY_TOKEN", "prompt": "Token for ghcr.io", "kind": "secret"}
```

When the step comes up, KeyJutsu writes one command directly (it is never
performed, since §25 turns staged typing off for credential flows):

- `secret`: `$REGISTRY_TOKEN = Read-Host -AsSecureString -Prompt 'Token for ghcr.io'`
- `username_and_password`: `$NAME = Get-Credential -Message '…'`, with
  `-UserName` when the plan suggests one.

The operator's keys then go straight to that prompt, which is PowerShell's
own. It shows `*` for each character, stores the value as a `SecureString` or
`PSCredential` in the session, and is not command-line input, so PSReadLine
does not record it. Later steps use the variable, for example
`[System.Net.NetworkCredential]::new('', $REGISTRY_TOKEN).Password | docker login … --password-stdin`.

Around that:

- **The step starts only on Enter.** Keys still being mashed for the previous
  step are swallowed, and no front end can start it for the operator
  (`Input::Start` is ignored for it). Otherwise mashed keys would land in the
  prompt.
- **Keys stop reaching the shell after the last answer.** The engine knows how
  many Enters the answer takes (one for a secret or a password; two for a
  user name and a password). After the last, it treats the step as a running
  command and swallows keys until the shell reports back, so nothing typed
  afterwards lands on the next prompt line.
- **The variable is removed when the plan ends** (complete or failed) with
  `Remove-Variable`. A resumed run asks again, because the variable lived in
  the shell that asked for it.
- **cmd is refused.** It has no masked prompt (`set /p` echoes), so validation
  marks a cmd credential step INVALID and preflight refuses it again.

## Consequences

- KeyJutsu never holds the secret: it passes keystrokes to the pseudo-console
  as it does for any typing, and keeps no record of input. The front end sees
  the keys because it is the keyboard; it sees only `*` in the output.
- Prompt text and user names come from the plan and go into a single-quoted
  PowerShell string. Every kind of single quote PowerShell recognises
  (including the typographic ones) is doubled, so a prompt cannot end the
  string (`the_credential_command_is_exactly_the_shells_own_masked_prompt`).
- A plan can still print the secret by using the variable carelessly (for
  example `…Password` on its own line). That is a plan that does what it
  says; validation does not yet flag it.
- Windows PowerShell 5.1's `Get-Credential` opens a Windows credential dialog
  instead of prompting in the console. That is Windows-native (§25), but it is
  not exercised by a test here.
- Windows Credential Manager, DPAPI-protected storage and SSPI are not used:
  nothing is stored, so there is nothing to protect. Storing a credential
  between runs would need them, and is not built.
