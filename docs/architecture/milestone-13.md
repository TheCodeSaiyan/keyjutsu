# Milestone 13: Git safety

What was built, and what the "done when" in §60 rests on.

## What it is

`keyjutsu-core::git`, used by `keyjutsu run`, `keyjutsu git diff` and the
desktop's plan runs.

- **Repository detection.** The repositories a plan works in: the one the
  shell starts in, and any named by a step's working directory.
- **A record before the run:** root, branch, HEAD, remotes, and every file
  that already differs from HEAD (modified, staged, deleted, untracked), with
  a SHA-256 of its content and a copy of it (files up to 16 MiB), kept next
  to the snapshot (`<snapshot>.git/`).
- **The comparison after the run.** A file whose status and content are
  exactly as recorded is the operator's change, untouched. Anything else that
  now differs is KeyJutsu's, including a file the operator had changed and a
  step changed again. When a step commits, the files in those commits are
  reported as KeyJutsu's too.
- **KeyJutsu's diff alone.** Each diff is taken against the file as it was
  just before the run (the operator's copy, or HEAD for a file that was
  clean, or nothing for a new file), never against HEAD alone, so the
  operator's own edits never appear as KeyJutsu's. `keyjutsu git diff
  <snapshot>` prints it; the desktop's run panel shows each file's diff.
- **Optional isolation:** `keyjutsu run --isolate worktree` runs the plan in a
  new worktree on a new local branch `keyjutsu/<plan>-<hash>`, leaving the
  working tree and its uncommitted changes alone; it refuses if a step names
  a folder inside the original repository, which a worktree elsewhere would
  not isolate. `--isolate branch` creates and switches to that branch in
  place. Without either, the plan runs in the current working tree (§31's
  three choices).
- **Nothing is committed or pushed by KeyJutsu itself.** Those are plan
  steps, approved like any other; creating the worktree or branch is the only
  Git write KeyJutsu makes, and only when asked.

## Done when

"KeyJutsu can modify a dirty repository and accurately distinguish its
changes from existing user changes."

`keyjutsu_changes_are_told_apart_from_the_operators_in_a_dirty_repository`
(real git, real pwsh, a throwaway repository):

- before the run, the operator has edited `a.txt`, deleted `c.txt` and added
  an untracked `u.txt`;
- the approved plan appends to the clean `b.txt`, appends to the
  operator's already-edited `a.txt`, and creates `k.txt`;
- the report says KeyJutsu changed `a.txt` (already changed by the operator),
  `b.txt` and `k.txt`, and that the operator's `c.txt` and `u.txt` are
  untouched;
- `a.txt`'s diff shows KeyJutsu's added line and not the operator's.

End to end through the CLI, `run_tells_its_git_changes_from_yours_and_shows_its_diff`
checks the printed summary and `keyjutsu git diff`.
`a_commit_made_by_a_step_is_reported_as_keyjutsus` covers a step that
commits; `a_worktree_isolates_a_run_from_the_operators_working_tree` covers
isolation.

## Checked by breaking it

- Comparing status without content: the operator's `a.txt`, still ` M`,
  was reported as untouched although a step had changed it. The test fails.
- Diffing against HEAD instead of the operator's copy: `a.txt`'s diff showed
  the operator's `+user-a` as KeyJutsu's. The test fails.

## Found on the way: working directories were ignored

A step's `working_directory` was validated and part of its hash, but the
executor never went there: the step ran wherever the shell happened to be.
Found while working out which repository a step touches. A step with a
working directory now starts with KeyJutsu's own `Set-Location -LiteralPath
'…'` (or `cd /d "…"` in cmd), sent directly rather than performed.
`a_step_runs_in_its_working_directory` uses a folder with a space and an
apostrophe in its name; without the fix the file was written to the shell's
starting folder (the test's own stray file there was removed afterwards).
The location is not restored after the step, so later steps without a
working directory run where the last one left the shell.

## Limits

- **KeyJutsu's Git changes are reported, not undone.** Undoing a step is
  Milestone 11's recovery, which covers what a step declares it captures; a
  "restore KeyJutsu's changes to this repository" action is not built.
- **Large files** (over 16 MiB) are hashed, so attribution is exact, but not
  copied, so a diff of KeyJutsu's part cannot be shown for them.
- **A file changed and changed back** during the run shows as untouched.
- **The worktree is not removed afterwards**; `git worktree remove` does it.
- **`--isolate` is not offered in the desktop yet**, and the CLI flag is only
  covered through the core functions, not end to end.
- **Submodules and ignored files** are not examined.
