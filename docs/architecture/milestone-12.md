# Milestone 12: network and artifact contracts

What was built, and what the "done when" in §60 rests on.

## What it is

- **Network destination detection** (`keyjutsu-validation::network`). Each
  command, visible check and recovery line is read for the hosts it names:
  URLs (HTTP, HTTPS, SSH, Git, SMB, FTP), UNC shares, `user@host:`
  addresses, `ssh` targets and `-ComputerName`. Validation compares them with
  the step's declared `network.destinations`: an undeclared host puts the step
  in review, and so does a host declared only for staging (`at_runtime:
  false`) that a command contacts while running.
- **Artifact pinning.** An artifact without a `sha256` puts its step in
  review: a moving target cannot be approved. Artifacts are handed to
  PowerShell steps only; one on a cmd step is INVALID.
- **Staging** (`keyjutsu-core::artifacts`; `keyjutsu plan stage [--pin FILE]`;
  "Stage downloads" in the desktop workspace). Each artifact is downloaded
  into a content-addressed store (`%LOCALAPPDATA%\KeyJutsu\artifacts\<sha256>\<name>`),
  hashed, compared with the pin, and kept with a record of its source,
  version, publisher, size, time and whether the plan had pinned it. A
  download that does not match its pin is deleted, never kept. Where the
  plan left an artifact unpinned, staging can write the hash into a copy of
  the plan (CLI) or into the draft (desktop), which is an edit: the step goes
  back to validation and the operator reviews the hash.
- **Execution uses the staged copy.** Arming refuses a plan whose artifacts
  are not all staged and verified. Just before a step runs, its artifacts are
  hashed again and their paths handed to it with KeyJutsu's own line,
  `$KJ_ARTIFACTS = @{ 'name' = '…' }`, sent directly rather than performed.
  Nothing is downloaded while a plan runs.
- **Agents are told the contract.** The ground rules now say to list
  downloads as artifacts, use `$KJ_ARTIFACTS['name']`, and declare every host.

## Done when

"A download-dependent task can be fully staged before arming and uses the
verified staged artifact during execution."

`a_download_dependent_task_runs_the_verified_staged_copy`, with a small HTTP
server on 127.0.0.1 standing in for the internet:

- unpinned, the plan cannot be approved (review);
- staging downloads it once, keeps it under its hash with its provenance, and
  pins the hash;
- the pinned plan validates READY and is approved;
- the server then serves different content, a moving target;
- the plan runs: the installed file is the staged, approved version 1, and
  the server received no request at all while it ran.

With it: `a_staged_copy_that_changed_stops_the_plan` (the staged file is
edited after approval; the run stops before the step, nothing is installed),
`a_plan_that_was_never_staged_does_not_arm` (and arming does not download),
`a_download_that_does_not_match_its_pin_is_not_kept`, and
`staging_in_the_workspace_pins_the_hash_and_asks_for_validation_again`.

## Checked by breaking it

- Skipping the hash comparison when the staged copy is used: the tampered
  copy was installed. The test fails.
- Skipping the comparison with the pin when staging: a mismatched download
  was kept. The test fails.

## Limits

- **Detection reads text.** A host assembled at run time (`"https://" +
  $name`) is not seen; nor is a program that contacts the network on its own
  (an installer's updater, `winget`, `npm install`). The finding is a floor,
  and says so in the evidence.
- **No publisher signature check.** The pin is a SHA-256; Authenticode
  signatures of executables are not checked yet.
- **Downloads go through PowerShell's `Invoke-WebRequest`**, so proxies and
  certificates follow the machine's settings; there is no size limit beyond
  the five-minute timeout.
- **Plain HTTP is allowed only from this machine**
  ([deviation D24](deviations.md#d24-artifacts-from-a-local-mirror-over-plain-http)).
- **The store is never cleaned.** Staged copies stay until deleted by hand.
- **Only PowerShell steps receive artifacts.**
