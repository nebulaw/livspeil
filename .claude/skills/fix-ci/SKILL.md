---
name: fix-ci
description: Diagnose and fix a failing CI check on a Livspeil pull request, push the fix and confirm the checks go green.
argument-hint: "<pr-number>"
disable-model-invocation: true
---

Fix CI on PR #$ARGUMENTS.

1. Head SHA and branch:
   `gh api repos/nebulaw/livspeil/pulls/$ARGUMENTS --jq '.head.sha,.head.ref,.mergeable_state'`
2. Check runs:
   `gh api repos/nebulaw/livspeil/commits/<sha>/check-runs --jq '.check_runs[]|"\(.id) \(.name) \(.status) \(.conclusion)"'`
   A still-running job is not a failure; say so and wait.
3. Failing job log, to a file, then search it:
   `gh api repos/nebulaw/livspeil/actions/jobs/<id>/logs > /tmp/job.log`
   `grep -nE 'error(\[|:)|panicked|FAILED|SIGSEGV|##\[error\]' /tmp/job.log | head -40`
   Then read the lines just before the first error.
4. Classify and fix the cause:
   - **New lint or rustfmt** (CI runs latest stable, newer than local):
     fix the code; never `allow` a lint without a reason in a comment.
   - **MSRV job** (Rust 1.95): an API newer than the MSRV; use an older
     equivalent.
   - **Platform-only** (macOS temp dirs, libm float digits, Linux system
     packages): make the test robust or add the package to both CI jobs.
   - **Crash or flake**: reproduce locally in a loop; find the cause (for
     example the lavapipe GPU segfault came from parallel device
     creation). Re-running alone is not a fix.
   - **Lockfile or merge damage** (often from the GitHub web editor):
     regenerate from main's `Cargo.lock` and rerun the check.
5. Run the affected crate's tests locally, then `cargo xtask check`.
6. Commit (explain the cause in the message), push to the PR branch, wait
   about 7 minutes, and re-check the runs until all are green.
7. Tell the user in two or three lines: what failed, the cause, the fix,
   and the current check status.
