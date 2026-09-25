# Milestone 15: session boundaries

What was built, and what the "done when" in §60 rests on.

## What it is

Plans could already be written in phases with a boundary after a phase
(`windows_restart`, `sign_out`, `shell_restart`, `wsl_restart`,
`docker_restart`), and the structure check enforced their order. Now the
executor honours them (`keyjutsu-core::boundary`).

- **Stop at the boundary.** When the next step belongs to a phase after an
  uncrossed boundary, the run stops with the outcome `Boundary`, and the
  checkpoint records what should be different on the far side: Windows' last
  boot time, the logon id (`whoami /logonid`), the shell's process id, or
  WSL's boot id. Docker exposes nothing that changes on restart, so for it
  the operator's word is all there is, and the notice says so.
- **Resuming checks, in this order, before asking anyone:**
  1. the boundary happened: the recorded identity must have changed, or the
     run stays stopped ("the Windows restart has not happened yet");
  2. the machine still matches the approved one: the environment is
     fingerprinted again, and drift affecting any remaining step stops the
     run with the steps that require revalidation;
  3. what the earlier phases achieved still holds: every state check of
     every completed step (paths, services, file hashes, JSON values, ports,
     HTTP) is run again, and a failure stops the run;
  4. the operator confirms. Without a confirmation the plan never resumes past
     a boundary. The CLI asks for `RESUME` typed on the plain console before
     the session starts, never a single key (§32).
- **Nothing from before the boundary is carried over except results that were
  checked again.** The shell-restart test shows a variable set in phase 1 is
  gone in phase 2, and nothing depended on it.
- **`keyjutsu run` says what to do at a boundary**: restart Windows, sign out,
  restart WSL or Docker, then `keyjutsu run <snapshot> --resume`.

## Done when

"A test Technique can: Phase 1 → controlled restart → resume → verify
environment → Phase 2, without assuming pre-restart state remains valid."

- `a_plan_crosses_a_real_shell_restart_without_trusting_the_old_shell`: a
  two-phase plan runs phase 1 in one real pwsh session and stops at the
  boundary; resuming in the same shell is refused; in a new shell it is
  checked (the marker from phase 1 is found again), confirmed and completed,
  and phase 2 reports that the old shell's variable is gone.
- `a_windows_restart_is_required_confirmed_and_followed_by_a_fresh_look_at_the_machine`:
  the same plan with a Windows restart, its boot identity injected. Before the
  restart it will not resume; after a restart that changed PowerShell it
  stops with phase 2 requiring revalidation; after a clean restart it will not
  resume without a confirmation, or with a refusal; confirmed, it completes.
- `what_phase_one_achieved_is_checked_again_after_the_boundary`: the marker
  phase 1 made is deleted across the boundary; resuming stops with "no longer
  holds", phase 2 does not run, and the operator is never asked to approve the
  broken state.

**A real Windows restart**, in Windows Sandbox (25 September 2026,
`tests/e2e/restart-trial/run.ps1`, networking off, only a staging folder
mapped): phase 1 ran and stopped at the boundary with the boot time
`2026-09-25T16:39:52.998Z`; resuming at once was refused ("the Windows
restart has not happened yet"); the Sandbox restarted; resuming without a
confirmation was refused; with one, the notice said `verified: true`, the
marker from phase 1 was rechecked and found, there was no drift, and phase 2
completed. Preparing it found that the restart check called PowerShell 7,
which Sandbox (and many machines) does not have, so the check would have
silently found nothing and resuming would have rested on the operator's word;
it now uses Windows PowerShell, which every Windows has.

A Technique produces an ordinary plan (Milestone 14), so these plans stand in
for a Technique's; no Technique-specific code is involved in crossing a
boundary.

## Checked by breaking it

Each of these made a test fail: resuming without checking the identity
changed; skipping the recheck of earlier results; resuming without a
confirmation.

## Limits

- **KeyJutsu does not restart Windows itself, and does not resume at sign-in**
  ([deviation D27](deviations.md#d27-the-operator-crosses-the-boundary-keyjutsu-checks-it)).
  The operator crosses the boundary and runs the resume.
- **A sign-out has not been crossed for real**; its identity (`whoami
  /logonid`) is the real one but only injected in tests. The shell restart
  and a Windows restart have been (below).
- **The desktop app stops at a boundary but cannot resume past one yet**;
  the CLI can.
- **Checks that only an exit code proves** (such as "the command succeeded")
  cannot be re-run after a boundary and are not repeated.

## Also seen

One full-suite run failed `a_trailing_comment_cannot_turn_a_dry_run_into_a_real_one`
(Milestone 5) at its second assertion: the file was not deleted, which is the
safety property it exists for, but the dry run did not report a pass. It did
not recur in eight runs of that test binary or in the next full-suite run
(437 passed). The failure output was not kept, so the cause is not known;
load on the machine at the time is the likeliest explanation, not a proven
one.
