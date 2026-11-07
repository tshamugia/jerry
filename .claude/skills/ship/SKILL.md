---
name: ship
description: End-of-issue workflow for Jerry — verify the work (pnpm check, bindings, the issue's acceptance items) and fix until green, commit with Conventional Commits, push the branch, open the PR with `Closes #N`, then merge it right away and delete the branch, as CLAUDE.md's Workflow section requires.
argument-hint: "[issue number and notes, e.g. 39 or Closes #39 #40]"
disable-model-invocation: true
---

# /ship — verify, commit, push, open and merge the PR

Run this when an issue's work is done. It carries out the Workflow section of `CLAUDE.md`; where the two
disagree, `CLAUDE.md` wins and this file gets fixed. Work through the steps in order and don't stop until the
PR is merged, unless a stop condition below applies.

Notes from the user (may be empty, e.g. the issue number): $ARGUMENTS

## 1. Orient

- `git fetch origin`, `git status`, `git branch --show-current`, `git log --oneline origin/main..HEAD`,
  `git diff --stat`. Note whether this is a linked worktree (`git rev-parse --git-dir` differs from
  `git rev-parse --git-common-dir`): step 7 depends on it.
- Nothing changed and nothing ahead of `origin/main`: say there is nothing to ship and stop.
- Find the issue from the notes above, the branch name or the commits, and read it with `gh issue view <N>`,
  above all its "მზადაა, როცა" (acceptance) items. Repo-wide docs or tooling work may ship without an issue
  (as #102 and #103 did).
- Branch: `<type>/<issue>-<slug>` from `main` (`feat/39-context-menu`), or `<type>/<slug>` without an issue.
  On `main`, create it with `git switch -c`; on a worktree's `claude/*` branch, rename it with `git branch -m`.
  Never commit to `main`.
- `spike/*` branches never merge into `main` (their results go into the Tech PRD): say so and stop.
- A release branch (`chore/release-vX.Y.Z`) also follows "Releases" in `CLAUDE.md`: ship its PR here, but push
  the tag only after the user says so.
- Behind `origin/main`: merge it in (`git merge origin/main`; in a desktop-app worktree use the app's sync with
  the base branch), resolve conflicts, then verify. Never rebase a pushed branch.
- Files that must never be committed: `.env*` with real values, `*.key`, `*.pfx`, `target/`, `dist/`,
  `.claude/worktrees/`, `.claude/settings.local.json`, scratch scripts and screenshots. Make sure they're
  git-ignored; never stage them.
- If `pnpm tauri dev` runs from this checkout, stop it first: it rewrites `src/ipc/bindings.ts` on Rust
  changes, and Windows can't replace a running `jerry.exe`.

## 2. Verify — loop until everything is green

Run both; after any fix, run both again from the top:

```bash
pnpm check      # Biome, rustfmt, clippy -D warnings, tsc, Vitest, nextest
pnpm bindings   # then `git status --porcelain src/ipc/bindings.ts` must print nothing
```

- `pnpm check` takes minutes (clippy, nextest): run it in the background and wait for it to finish.
- A diff after `pnpm bindings` means the committed bindings were stale. The regenerated file is the fix:
  commit it with the change that caused it.
- When the change touches them, also:
  - Tauri config, capabilities, the manifest, `build.rs`, installer hooks or release settings:
    `pnpm tauri build --no-bundle` (`pnpm build` when the installer changed).
  - A path with a performance budget (listing, sort, quick filter, search, scrolling): `cargo bench -p jerry-core`
    or `pnpm bench`, compared with the budgets in `CLAUDE.md`. A change that misses a budget isn't done.
- Rules:
  - Fix the code, not the check. Nothing added just to get green: no `#[allow]`/`#[expect]`, `biome-ignore`,
    `@ts-ignore`/`@ts-expect-error`, `as any`, `#[ignore]` or `.skip`, no deleted or weakened tests, no
    `--no-verify`.
  - Never weaken a data safety rule to make a test pass.
  - Stop and ask the user if the same failure survives three honest fix attempts, or if the fix needs a product
    or design decision.
  - Report only what ran. A check that didn't run is not passing.

## 3. Acceptance

An issue is done when its "მზადაა, როცა" items are verified, not when its checklist is ticked.

- For each item, note how it was verified: a test (by name), a run of the dev app, a measurement.
- Items only a person can check (OLE drag & drop, the clipboard with other apps, OneDrive, network drives,
  Explorer side by side, the look against the design) need the user's confirmation. If one isn't confirmed yet,
  list exactly what to check and how, then stop and wait: no PR before the user confirms.
- If behavior, architecture or a rule changed: update the Claude Doc of the PRD or Tech PRD first, then
  re-export `docs/PRD.md` or `docs/TECH-PRD.md`; update `CLAUDE.md` if a rule or the structure changed, and the
  phase's manual checklist in `docs/checklists/` if it has one.

## 4. Commit

- Stage files by name, not `git add -A` or `git add .`, and read `git diff --cached --stat` before committing.
- Conventional Commits in English, in the style of `git log`: `<type>: <summary>` in lowercase
  (`feat: the file list's context menu, Show in Explorer and Open in Terminal`), type one of `feat`, `fix`,
  `perf`, `refactor`, `test`, `docs`, `chore`, `ci`. The body says why.
- Make several atomic commits when the changes split naturally (core, UI, docs).
- In PowerShell 5.1, `git commit -F -` with a here-string doesn't reach stdin: write the message to a file
  and use `git commit -F <file>`.

## 5. Push

- `git push -u origin <branch>`. Never force-push; if the push is rejected, stop and report.

## 6. Open the PR

- `gh pr view --json number,url,state`: if this branch already has an open PR, the push updated it; go on to
  step 7 with it.
- Otherwise `gh pr create --base main --title "<type>: <summary>" --body-file <file>`. No draft PRs.
  - The title must pass `.github/workflows/pr-title.yml`:
    `^(feat|fix|perf|refactor|test|docs|chore|ci)(\([a-z0-9-]+\))?!?: [^ ]`.
  - The body follows `.github/pull_request_template.md`: a `Closes #N` line for every issue this finishes,
    **What** (what changed and why), **Verified** (each check from step 2 with its result, each acceptance item
    with how it was verified and who confirmed the manual ones), and the **Before opening** checklist, ticked
    only where true.

## 7. Merge and clean up

Once the PR is open, merge it right away: a merge commit, no review or CI wait. Then delete the branch on
GitHub and locally.

- Main checkout: `gh pr merge <n> --merge --delete-branch`.
- Linked worktree: `--delete-branch` can't switch the worktree to `main`, so run `gh pr merge <n> --merge`,
  `git push origin --delete <branch>`, `git fetch origin`, `git switch --detach origin/main` and
  `git branch -d <branch>`.
- Then confirm:
  - `gh pr view <n> --json state` says `MERGED`.
  - `git ls-remote --heads origin <branch>` prints nothing.
  - Each issue in `Closes` is closed (`gh issue view <N> --json state`). A merge hasn't always closed it
    (#135 left #38 open): close it with `gh issue close <N> --comment "Done in #<n>."`.
- If issues were added or reordered, update the epic's task list.
- CI still runs on the PR and on `main` as a safety net. Don't wait for it; if it turns red, the fix comes
  next, in its own PR.

## 8. Report

Reply with the PR link and its merge commit, the commits, each check with its result, how each acceptance item
was verified, and anything left over (an issue closed by hand, CI still running on `main`).
