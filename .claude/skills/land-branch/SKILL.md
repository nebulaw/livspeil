---
name: land-branch
description: Merge main into a finished Livspeil branch, run the full check, look at its output, push it and open a pull request.
argument-hint: "<branch>"
disable-model-invocation: true
---

Land branch `$ARGUMENTS`.

1. Find its worktree: `git worktree list`. Work there.
2. `git status` must be clean; if the agent left uncommitted work, read it
   and commit it with a clear message (or ask the agent to).
3. `git fetch origin && git merge origin/main`. Resolve conflicts
   yourself; for `Cargo.lock` take main's version and let cargo update it.
4. `cargo xtask check` must pass. Read failures with `grep`, not by
   dumping whole logs.
5. **Look at the output.** Open every new or changed golden PNG
   (`git diff --name-only origin/main...HEAD -- '*.png'`) with the Read
   tool, and run any CLI example the change affects. Fix or send back
   what looks wrong.
6. For high-stakes changes (kernel semantics, file formats, licensing,
   sandboxing) run the `reviewer` agent on the branch first.
7. Push: `git -c push.negotiate=true push -u origin <branch>`.
8. Open the PR with the REST API (GraphQL may be unavailable):
   `gh api repos/nebulaw/livspeil/pulls -f title='…' -f head=<branch> -f base=main -F body=@/tmp/pr.md --jq .html_url`.
   Body: **Summary** (what and why, bullets), **Tests**, **Not verified**,
   **Known gaps**, then the tool attribution footer.
9. Delete the worktree's `target/` to save disk.
10. Report the PR link to the user with one line on what it delivers and
    send the most telling screenshot. Watch CI; on failure follow
    `/fix-ci`.
