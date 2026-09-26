# 0017: The broker captures and restores what an Administrator step changes

Status: proposed. Nothing here is built yet; until it is, restoring an
Administrator step's captures needs KeyJutsu run as Administrator.

## Context

A step can declare what it will change (a file, a registry value, a
service's state) and ask for it to be captured just before it runs, so it
can be put back after a failure. Captures are taken by KeyJutsu itself and
kept beside the run's checkpoint, in the operator's profile: the file
backups in the run's recovery folder, the rest in the checkpoint. The
checkpoint is recorded in the encrypted store, so an edited one is refused
([ADR 0016](0016-encrypted-file-store.md)).

That is enough while the captures are restored by the operator's own,
unelevated KeyJutsu: whatever could forge them could make the same changes
itself. It is not enough for an Administrator step. Its captures are of
things only an Administrator can change back, so restoring them means
writing as Administrator. If the elevation broker did that from captures in
the profile, anything running as the operator could forge a capture (it can
ask DPAPI to record a checkpoint just as KeyJutsu can) and have the broker
write chosen values to the registry or chosen bytes to a file, as
Administrator. The broker would have become a way round UAC, which is the
one thing [ADR 0011](0011-elevation-broker.md) says it must never be.

So today the broker recovers an Administrator step only by running that
step's own approved recovery commands, and a step that relies on captured
state is marked "cannot be recovered" unless KeyJutsu itself is elevated.

## Decision

The broker takes an Administrator step's captures itself, keeps them where
only Administrators can write, and is the only thing that restores them.

- **Captured by the broker.** When the broker runs an Administrator step
  whose recovery is `restore_captured_state`, it captures what the step
  declares, from its own verified copy of the snapshot, before running the
  step. If a capture cannot be made and verified, the step does not run,
  as for any other step.
- **Kept in `%ProgramData%\KeyJutsu\captures\<snapshot hash>\<step>`.** The
  installer creates `%ProgramData%\KeyJutsu` with an access list of
  Administrators and SYSTEM, full control, nothing inherited. An ordinary
  user can create folders in `%ProgramData%`, so one could make
  `KeyJutsu` there first and own it: the broker therefore checks, every
  time, that the folder's owner is Administrators or SYSTEM and that no one
  else may write to it, and refuses to capture or restore otherwise.
- **Restored only by the broker, only what the step declared.** A third
  request, `RestoreStep`, names the snapshot, the step and its hash, like
  the other two; it carries no path and no value. The broker restores the
  targets its own snapshot declares for that step, from its own captures,
  and checks each against what was captured.
- **The operator still sees the plan.** The broker's answer to a step says
  what it captured, as text for the recovery plan. KeyJutsu keeps that in the
  checkpoint for display only; nothing in it is used to restore.
- **Captures are removed** by the broker once the step's recovery has
  succeeded, when a later run of the same snapshot completes, and by the
  uninstaller. The broker also removes captures more than 30 days old each
  time it starts.
- **The portable build is unchanged.** It has no broker, and so no
  Administrator steps.

## Alternatives

- **Sign the captures.** A key the broker signs with has to be kept
  somewhere only Administrators can read, which is the same storage
  problem, with a second mechanism on top.
- **DPAPI with the machine key.** Any process on the machine can encrypt
  with it, so it proves nothing about who wrote a capture.
- **The Windows folder's `Temp`**, where the broker hands a step its
  artifacts. Windows' own clean-up empties it after some days, and a
  recovery can be wanted later than that.
- **Leave it as it is.** Recovery through a step's own approved commands
  covers most Administrator steps; a step that relies on captured state asks
  for KeyJutsu to be run as Administrator. This is the honest fallback, and
  the one in place until this is built.

## Consequences

- The broker writes to disk for the first time outside a step's own
  commands, so its tests have to cover the ownership check: a folder
  pre-created by an ordinary user is refused.
- The protocol goes to version 3.
- A capture now lives in two places: the broker's store, which is the
  truth, and the checkpoint's copy for display. They are never reconciled;
  only the broker's is used.
- Recovery of an Administrator step works only where the broker is
  installed, and after an uninstall its captures are gone.
