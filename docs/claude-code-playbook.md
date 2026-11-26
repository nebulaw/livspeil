<!--
SPDX-FileCopyrightText: 2026 Livspeil contributors
SPDX-License-Identifier: CC-BY-4.0
-->

# Building Livspeil with Claude Code

This is how Phase 0 and Phase 1 were built: one Claude session acts as
product owner and lead engineer, several Claude agents implement scoped
milestones in parallel git worktrees, and every change lands through a
reviewed, CI-green pull request. Everything here works on your own Linux
(or macOS) machine with Claude Code in the terminal.

The repository already carries the setup:

| Path | What |
|---|---|
| `CLAUDE.md` | Hard rules every session and agent reads first |
| `.claude/settings.json` | Shared permissions: what runs without asking, what always asks |
| `.claude/agents/implementer.md` | Subagent that builds one milestone in its own worktree |
| `.claude/agents/reviewer.md` | Read-only subagent that reviews a branch against the spec |
| `.claude/skills/start-round/` | `/start-round`: plan parallel tracks and launch implementers |
| `.claude/skills/land-branch/` | `/land-branch <branch>`: merge main, check, look at output, push, open the PR |
| `.claude/skills/fix-ci/` | `/fix-ci <pr>`: read the failing job log, fix, push, watch |
| `specs/`, `docs/` | The specs the work is checked against, and as-built docs |

## 1. One-time setup (Arch Linux)

```sh
sudo pacman -S --needed base-devel git github-cli rustup python \
    fontconfig vulkan-icd-loader vulkan-tools reuse libxkbcommon
# plus the Vulkan driver for your GPU: vulkan-radeon, vulkan-intel or nvidia-utils
rustup default stable            # rust-toolchain.toml pins the channel
cargo install cargo-deny --locked
gh auth login                     # HTTPS, with repo and workflow scopes
```

Install Claude Code by following <https://code.claude.com/docs/en/setup>,
then sign in with `claude` and run `/status` once to confirm the account
and that `.claude/settings.json` loaded (`Setting sources` line).

Check the repo builds before you start any session:

```sh
git clone git@github.com:nebulaw/livspeil.git && cd livspeil
cargo xtask check                 # fmt, clippy -D warnings, tests, deny, reuse
cargo run -p livspeil-ui-gallery  # the component gallery in a real window
```

Disk: each worktree has its own `target/` (several GB for the UI crates).
Keep about 30 GB free when three agents run at once. Agents export
`CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0`, which cuts this roughly
in half; delete a worktree's `target/` as soon as its PR is open.

## 2. The roles

**You** decide: product direction, open questions, licences, and merging.
Decisions are numbered (D1…D50 so far) and recorded in the project hub
doc. A session never merges to `main`, never pushes to `main`, and never
merges from the GitHub web editor (that corrupted `Cargo.lock` once; ask
the lead session to resolve conflicts instead).

**The lead session** (the Claude you talk to) plans rounds, writes the
agent briefs, reviews and test-merges branches, looks at real output
(screenshots, CLI output), pushes branches, opens PRs, fixes CI and keeps
the docs as-built. It writes little code itself, so its context stays free
for judgment across the whole round.

**Implementer agents** each own one milestone in one worktree. They read
the spec, build with tests, run `cargo xtask check`, look at what they
produced, commit in small steps and report. They never push.

**The reviewer agent** (optional, for high-stakes work) reads a finished
branch cold against its spec, so the work is not grading itself.

## 3. A round, step by step

1. **Decide.** Settle open questions before code. Ask the lead to
   "run a decision round" when several are pending: options, trade-offs,
   a recommendation each, and you answer. Answers become numbered
   decisions in the hub doc.
2. **Plan tracks.** `/start-round M1.11 M1.12` (or describe the goal).
   The lead splits the work into tracks that touch disjoint files where
   possible, writes one self-contained brief per track and launches an
   implementer for each, in the background, each in
   `../wt/<track>` on its own branch from `origin/main`.
3. **Wait cheaply.** The lead does not poll. You are free to talk to it
   about something else; agent reports arrive as notifications. If a usage
   limit interrupts an agent, say "resume" after the reset: the lead
   resumes the same agent with its context rather than starting over.
4. **Land.** For each finished track: `/land-branch p1/m1.11-foo`. The
   lead merges `origin/main`, runs the full check, opens the new golden
   images and looks at them, writes the PR body (summary, tests, what was
   not verified, known gaps) and opens the PR.
5. **CI.** If a check fails, say "PR 37 is failing" or run `/fix-ci 37`.
   The lead reads the failing job's log, reproduces locally, fixes the
   cause (not the symptom), pushes and waits for green.
6. **Merge.** You merge on GitHub. Tell the lead "merged"; it cleans up
   worktrees and starts the next step.
7. **Close out.** After a round: "close out the round". The lead syncs
   the as-built specs with `main`, bumps the hub doc revision, updates
   roadmap and risks, and lists the next open questions.

## 4. Writing a brief that works

A background agent starts cold. The brief is everything it knows, so the
lead writes it like a ticket for a strong engineer who has never seen the
repo:

- **Where:** the exact worktree path and branch; "work only there".
- **Rules:** read `CLAUDE.md` first; licences; no `unsafe`; no
  `Signed-off-by`; the commit trailer; never push.
- **Context:** which crates and docs to read, what landed recently that
  it builds on (with names of the types and functions).
- **Tasks:** numbered, each with its acceptance test. Name the spec
  section. Say what is out of scope.
- **Verification:** `cargo xtask check` must pass; open every new golden
  image and fix what looks wrong; tests for every behaviour.
- **Report:** commits, what each part does, what was *not* verified (no
  display, no GPU), known gaps, numbers measured.

The implementer agent definition already carries the rules, so a brief
can focus on context and tasks.

## 5. Rules that keep quality high

- **Spec first.** Every milestone has a spec section. Behaviour the spec
  does not cover is decided by you, not guessed.
- **Look at real output.** Tests passing is not enough for UI: the lead
  and the agents open the rendered PNGs and the CLI output. Several real
  bugs (row vs column layout, a missing-glyph box, a clipped minimap)
  were only found by looking.
- **Say what was not verified.** Every report and PR body lists it; for
  example a real window, or a real screen reader.
- **Small PRs, one milestone each.** Disjoint files across parallel
  tracks; shared files (`Cargo.lock`, `README.md`, CI) are merged by the
  lead, never by hand on the web.
- **Fix causes.** A flaky crash in CI is investigated (the lavapipe GPU
  test segfault was parallel device creation), not just re-run.
- **CI runs newer Rust than you.** CI uses latest stable plus an MSRV
  job on exactly 1.95; new clippy lints appear there first.

## 6. Talking to the lead

Short messages work. These are the ones used most:

| You say | The lead does |
|---|---|
| "Let's go" / "resume" | Continues the current plan, or resumes interrupted agents |
| "PR 32 is failing" | Reads the job log, fixes, pushes, waits for green |
| "All merged" | Cleans up, starts the next track or round |
| "Run a decision round on X" | Options with trade-offs and a recommendation |
| "Close out the round" | As-built specs, hub doc, roadmap, next questions |
| "Show me" | Sends the latest screenshots |

Give direction, not implementation: "the canvas must stay at 120 fps with
2000 nodes" beats "add a spatial index". Say when you want speed over
polish; the default is reviewed and tested.

## 7. Limits and cost

- The lead keeps its own context small: it reads logs with `grep`, and
  lets agents do the heavy reading. When a session gets long it is
  compacted automatically and continues from a summary.
- Parallel agents multiply usage. Two or three tracks at a time is the
  sweet spot; more mostly creates merge work.
- When the weekly limit is near, tell the lead "usage is tight"; it
  will brief agents to finish and commit before adding extras.
- Prefer resuming an interrupted agent over launching a new one: the
  new one would re-read everything.

## 8. Permissions

`.claude/settings.json` lets sessions run cargo, read-only git and
`gh` reads, and file edits without asking. Pushing, `gh api` writes,
deleting files and destructive git (`reset --hard`, `clean`, force push)
always ask. Put personal extras in `.claude/settings.local.json` (git
ignores it). Auto mode is set per user, not in the repo; if you turn it
on, its classifier still blocks destructive actions, and an agent that is
blocked stops and reports instead of working around it.

### Personal settings that matter

Put these in `~/.claude/settings.json` (yours, not the repo's):

```json
{
  "model": "claude-opus-5-5",
  "effortLevel": "high"
}
```

- The lead session benefits most from the strongest model and high
  effort: it plans, reviews and catches what agents miss. Agents inherit
  the model (`model: inherit` in `.claude/agents/implementer.md`).
- Start the lead from the main checkout (`cd livspeil && claude`) so it
  reads `.claude/settings.json`, `CLAUDE.md` and the skills.
- Your claude.ai account skills (`round-closeout`, `decision-round`,
  `spec-first-task`) are available in Claude Code too when skill sync is
  on; they cover closing a round, running a decision round and a single
  spec-first task.
- Attach the claude.ai project "Livspeil" when you work from claude.ai,
  so the hub doc and specs are in reach; in the terminal, the same specs
  are in `specs/`.

## 9. Where things are written down

| Where | What |
|---|---|
| Project hub doc (claude.ai project "Livspeil") | Vision, numbered decisions, roadmap, risks |
| `specs/` | Kernel and UI specs, kept as-built |
| `docs/` | Component docs, known upstream issues, benchmarks |
| PR bodies | What changed, what was verified, known gaps |
| `docs/known-issues.md` | Upstream bugs and the workarounds in our code |
