---
name: start-round
description: Plan a Livspeil work round into parallel tracks and launch one implementer agent per track in its own git worktree.
argument-hint: "[milestones or goal, e.g. M1.11 M1.12]"
disable-model-invocation: true
---

Start a work round for: $ARGUMENTS

1. **Ground it.** `git fetch origin`. Read the spec sections for these
   milestones (`specs/`), the hub doc's open decisions if the project is
   attached, and what recently landed (`git log --oneline -15 origin/main`).
   If a decision the work depends on is open, ask the user first (one
   short question with options and your recommendation) and stop.
2. **Split into tracks.** Two or three tracks, each one PR, touching
   disjoint files where possible. Note shared files (`Cargo.lock`,
   `README.md`, CI, `docs/widgets.md`) — you merge those, not the agents.
3. **Make worktrees.** For each track:
   `git worktree add -b <branch> ../wt/<track> origin/main`
   Branch names: `p<phase>/m<milestone>-<slug>`, for example
   `p1/m1.11-gallery-containers`. Check `df -h` first: about 10 GB per
   track.
4. **Brief and launch.** Launch the `implementer` agent once per track,
   in the background. Each brief is self-contained: worktree path and
   branch, the commit trailer including
   `Claude-Session: ${CLAUDE_SESSION_ID}` if a session URL is known,
   context (crates, docs, types it builds on), numbered tasks with
   acceptance tests, what is out of scope, and the report format.
5. **Tell the user** in two or three lines: the tracks, their branches,
   and what each will deliver. Then wait for the agents' reports; do not
   poll them.

When reports arrive, land each branch with `/land-branch <branch>`.
