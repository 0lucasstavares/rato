# RATO Autonomous GitHub Workflow

This document describes the live autonomous workflow used to build RATO through
GitHub issues, pull requests, Actions, and the shell dashboard.

## Control Plane

GitHub is the control plane.

- Issues are the backlog.
- Labels are routing and risk state.
- Pull requests are implementation logs.
- GitHub Actions runs are the agent execution trace.
- The shell dashboard `Agents` tab observes public GitHub issues, PRs, and
  Actions runs.

Human input should be limited to prompts, issue comments, labels, priorities,
and workflow dispatches. Agents are expected to create issues, write code,
review, merge, and create follow-up work.

## Autonomy Switch

Autonomous scheduled runs are controlled by the repository variable
`RATO_AUTONOMY`.

- `autonomy-on` sets `RATO_AUTONOMY=on`.
- `autonomy-off` sets `RATO_AUTONOMY=off`.

Scheduled agent runs only execute while autonomy is `on`. Manual dispatches are
still allowed while autonomy is off.

## Workflow Chain

The chain is event-driven, not only cron-driven:

```text
agent-merger -> agent-manager -> agent-worker -> pull request
pull request -> ci + agent-reviewer -> agent-merger
```

The practical loop is:

1. `agent-manager` reads issues and PRs, classifies work, creates missing issues,
   and leaves `Agent Brief` blocks.
2. `agent-worker` takes ready work, edits the repository, and the harness
   commits/pushes any resulting diff to an `ai/worker/<run>` branch and opens a
   PR.
3. `ci` and `agent-reviewer` run on the PR.
4. `agent-merger` merges clean, green, reviewed PRs and closes the loop.
5. A successful merger triggers another manager pass.

If a worker run completes without a repository diff and there are still no open
pull requests, a post-worker workflow job re-dispatches `agent-manager`
to avoid a silent stall. This is a bounded handoff, not an unbounded recursive
loop.

## Roles

- `manager`: keeps the backlog alive. It labels issues, splits broad work,
  creates new issues from docs and discovered gaps, and writes `Agent Brief`
  blocks.
- `worker`: implements one primary issue per PR. It may create follow-up issues
  for discoveries that should not enter the current PR.
- `reviewer`: checks PRs against their issue, architecture, and CI. It may file
  non-blocking follow-up issues.
- `merger`: merges eligible PRs and leaves an `Agent Merge Decision`.

Role prompts live in `docs/agents/roles/`.

## Issue Creation

Agents are expected to create issues directly with `gh issue create` when they
discover missing work.

New issues should use:

- Title prefix: `AI discovered:`
- Exactly one routing label: `ai:inbox` or `ai:ready`
- Exactly one risk label: `risk:r0`, `risk:r1`, `risk:r2`, or `risk:r3`
- Exactly one type label: `type:bug`, `type:feature`, `type:test`,
  `type:docs`, `type:refactor`, or `type:chore`
- An `Agent Brief` block
- Evidence linking back to files, docs, commands, issues, or PRs

Use `ai:ready` only when the issue has enough acceptance criteria for a worker.
Use `ai:inbox` when the manager still needs to classify or decompose it.

## Worker Publishing

The worker model is not trusted to remember the GitHub mechanics. After a
successful worker run, `scripts/agent/run-agent-role.ps1` checks for repository
diffs.

If a diff exists, the harness:

1. Creates an `ai/worker/<run-id>-<attempt>` branch.
2. Stages and commits the diff.
3. Pushes the branch.
4. Opens a PR against `main`.

If there is no diff, the run may still have acted on issues or comments.

## Merge Policy

The live merger harness performs a deterministic eligibility pass before the
merger model prompt runs.

A PR is eligible when:

- It is open and not draft.
- It is mergeable or clean according to GitHub.
- It is not labelled `ai:blocked`.
- All check runs in the PR rollup are completed and successful or skipped.
- The `agent-reviewer` workflow has completed successfully.

When eligible, the harness comments with `Agent Merge Decision`, squash-merges
the PR, and deletes the branch.

The older prose policy requiring `ai:merge` and multiple written review blocks
is advisory context for agents, not the current deterministic gate. The current
system intentionally allows agents to merge green reviewed PRs without waiting
for human review.

## Harness Selection

Actions set `RATO_AGENT_COMMAND` to:

```text
pwsh ./scripts/agent/run-provider-agent.ps1
```

The role workflows run a harness matrix:

- `codex`: Codex CLI through `@openai/codex`
- `claude-code`: Claude Code CLI through `@anthropic-ai/claude-code`

Each matrix job sets `RATO_AGENT_HARNESS` and `RATO_AGENT_ID`, so prompts, logs,
and worker branches identify which harness acted. Worker branches include the
harness, for example:

```text
ai/worker/codex-<run-id>-<attempt>
ai/worker/claude-code-<run-id>-<attempt>
```

When `RATO_AGENT_HARNESS=auto` is used outside the matrix, the harness wrapper
chooses:

1. OpenAI/Codex when `OPENAI_API_KEY` or `CHATGPT_API_KEY` is present.
2. Anthropic/Claude Code when `ANTHROPIC_API_KEY` is present and OpenAI is not.

Preferred harness secrets in Actions:

- `RATO_CODEX_API_KEY`
- `RATO_CLAUDE_AUTH_TOKEN`

Fallback aliases still work:

- `OPENAI_API_KEY`
- `CHATGPT_API_KEY`
- `ANTHROPIC_API_KEY`
- `ANTHROPIC_AUTH_TOKEN`

Defaults:

- Primary model: `gpt-5.1-codex-max`
- Fast model: `gpt-5-mini`
- Review model: `gpt-5.1`
- Embedding model: `text-embedding-3-small`
- Audio model: `whisper-1`

## Observability

Open the shell dashboard in development:

```powershell
cd .\apps\shell
npm.cmd run dev -- --host 127.0.0.1 --port 19773
```

Then open:

```text
http://127.0.0.1:19773/dashboard.html
```

The `Agents` tab reads public GitHub data directly in browser mode:

- issue queue counts
- open PR count
- recent `agent-*` workflow status
- harness usage in a scrollable feed, with failed/cancelled Codex or Claude Code
  jobs shown red as quota/auth risk

Each worker run also writes an `Agent Outcome` block into the GitHub Actions
step summary so the Actions UI distinguishes:

- PR opened
- existing PR reused
- no repository diff
- staged diff collapsed to empty

If a future daemon RPC named `agents.observability` is available, the tab can use
that richer source instead.

## Operator Commands

Turn autonomy on:

```powershell
gh workflow run autonomy-on.yml --repo 0lucasstavares/rato
```

Turn autonomy off:

```powershell
gh workflow run autonomy-off.yml --repo 0lucasstavares/rato
```

Run a role manually:

```powershell
gh workflow run agent-manager.yml --repo 0lucasstavares/rato
gh workflow run agent-worker.yml --repo 0lucasstavares/rato
gh workflow run agent-reviewer.yml --repo 0lucasstavares/rato
gh workflow run agent-merger.yml --repo 0lucasstavares/rato
```

Inspect state:

```powershell
gh issue list --repo 0lucasstavares/rato --state open
gh pr list --repo 0lucasstavares/rato --state open
gh run list --repo 0lucasstavares/rato --limit 20
```

