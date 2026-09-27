# 0018: Copies of files are kept encrypted; plans and snapshots stay readable

Status: proposed, 27 September 2026.

## Context

The history and Techniques live in the encrypted store
([ADR 0016](0016-encrypted-file-store.md)). Several other things a run
leaves on disk do not, and sit in the operator's profile as plain files:

| What | Where | What it can hold |
| --- | --- | --- |
| Recovery backups | the run's recovery folder, `<step>-<n>.bak` | a whole copy of each file a step captured: a config with a connection string, an `.env` |
| Git copies | the run's `git` folder, named by hash | a whole copy of each file the operator had changed before the run, taken so KeyJutsu's diff can be told from theirs |
| Git baseline | `git\baseline.json` | paths, hashes and HEADs, no contents |
| Sealed snapshot | `snapshot.json`, or wherever `--out` put it | the approved plan: commands, paths, checks |
| Checkpoint | `snapshot.checkpoint.json` | which steps ran, their exit codes and check results, capture records |
| Plan files | wherever the operator or agent wrote them | the draft plan |

Integrity is already covered: an edited snapshot or checkpoint is refused,
because its hash is recorded in the store; recovery backups are checked
against the hash recorded when they were taken; the Git record is checked
before use. What is not covered is confidentiality. The specification asks
for prompts, plans, outputs, snapshots and history to be encrypted at rest.

Two kinds of thing are mixed together here. Recovery backups and Git copies
are **the contents of the operator's files**, copied by KeyJutsu into a
second place the operator did not choose, and may hold anything those files
did, secrets included. Snapshots, checkpoints and plans are **KeyJutsu's own
records of a plan**: commands and paths the operator reviewed, which the CLI
takes by path (`keyjutsu run`, `plan verify`, `plan diff`, `recover`), which
are meant to be read, diffed and kept beside a project, and which never
hold a credential (credentials are typed into the shell's masked prompt and
never kept; a failed step's output is redacted before it reaches the
history).

## Options

1. **Encrypt everything.** Snapshots, checkpoints, plans and copies all move
   into the store; the CLI names them by id rather than by path. The most
   complete, and the largest change: every command that takes a snapshot
   path changes, plans can no longer be reviewed or diffed as files, and a
   plan written by an agent arrives as a file whatever KeyJutsu does.
2. **Encrypt file contents only.** Recovery backups and Git copies are
   written encrypted with the store's key (AES-256-GCM, the key
   DPAPI-protected for this user on this machine, as for every record), and
   read back through the store. Snapshots, checkpoints, baselines and plans
   stay readable files, integrity-checked as now. The broker's own captures
   stay where they are: in `%ProgramData%\KeyJutsu`, which only
   Administrators and SYSTEM can read, and which the operator's key could not
   protect from the operator anyway.
3. **Leave it.** Record the gap as a deviation.

## Decision (proposed)

Option 2. The copies are the only thing on the list that holds data
KeyJutsu did not show the operator, and they are copies the operator did
not ask for; that is where encryption at rest changes something.
Snapshots and plans are deliberately readable, and their integrity is
already enforced; encrypting them would cost the CLI's file-based workflow
for little gain. This is recorded as a deviation from the specification's
"snapshots and plans encrypted", with the reason above.

## Consequences

- A backup or Git copy made on one account or machine cannot be read on
  another, like everything in the store. Recovery and `keyjutsu git diff`
  on the machine and account that ran the plan are unchanged.
- Backups already on disk from earlier runs are read as before (plain) and
  are not rewritten; new ones are encrypted.
- A test looks for each copied file's contents in plain text on disk after
  a run, as the store's own test does for records.
