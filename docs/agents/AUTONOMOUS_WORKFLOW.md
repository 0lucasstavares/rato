# RATO Autonomous Workflow

This document describes the live autonomous workflow used to build RATO through
GitHub issues, pull requests, GitHub Actions role execution, and optional local
operator tooling.

## Control Plane

GitHub is the shared state and GitHub Actions are the primary autonomous
runner.

- Issues are the backlog.
- Labels are routing and risk state.
- Pull requests are implementation logs.
- CI remains the deterministic code-validation gate.
- GitHub Actions execute the role loop.
- The local supervisor and localhost dashboard are fallback operator tools.

Human input should be limited to prompts, issue comments, labels, priorities,
and occasional operator restarts or manual workflow dispatches. Agents are
expected to create issues, write code, review, merge, and create follow-up
work.

## Actions Loop

The live Actions workflows are:

```text
.github/workflows/agent-scrum-master.yml
.github/workflows/agent-manager.yml
.github/workflows/agent-worker.yml
.github/workflows/agent-reviewer.yml
.github/workflows/agent-merger.yml
```

The default loop is:

```text
scrum-master -> manager -> worker -> reviewer -> merger
```

The practical behavior is queue-driven:

1. `scrum-master` organizes the current queue and leaves the next handoff.
2. `manager` classifies issues, splits broad work, and prepares ready items.
3. `worker` takes ready work, edits the repository, and opens a PR when a diff
   exists.
4. `reviewer` inspects open PRs, requests fixes when needed, and adds
   `ai:merge` when the PR is acceptable.
5. `merger` merges clean green PRs that carry the reviewer handoff label.

If there is already an open PR, the workflow chain prefers review and merge
work instead of starting more implementation.

Scheduled and chained runs are gated by the `RATO_AUTONOMY` GitHub variable.
Use these manual control workflows:

```text
.github/workflows/autonomy-on.yml
.github/workflows/autonomy-off.yml
```

## Roles

- `scrum-master`: organizes the backlog and handoffs. It does not create sprint
  calendars, day plans, or week plans; it only decides what should move next.
- `manager`: keeps the backlog alive. It labels issues, splits broad work,
  creates new issues from docs and discovered gaps, and writes `Agent Brief`
  blocks.
- `worker`: implements one primary issue per PR. It may create follow-up issues
  for discoveries that should not enter the current PR.
- `reviewer`: checks PRs against their issue, architecture, and CI. It may file
  non-blocking follow-up issues and must add `ai:merge` only when the PR is
  actually mergeable.
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

The worker model is not trusted to remember the Git mechanics. After a
successful worker run, `scripts/agent/run-agent-role.ps1` checks for repository
diffs.

If a diff exists, the harness:

1. Creates an `ai/worker/<run>` branch.
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
- It carries the `ai:merge` label from the reviewer.
- All check runs in the PR rollup are completed and successful or skipped.

When eligible, the harness comments with `Agent Merge Decision`, squash-merges
the PR, and deletes the branch.

The current system trusts the reviewer handoff label plus green CI.

## Harness Selection

Each Actions workflow defaults `RATO_AGENT_COMMAND` to:

```text
pwsh ./scripts/agent/run-provider-agent.ps1
```

The provider wrapper supports:

- `codex`: Codex CLI through `@openai/codex`
- `claude-code`: Claude Code CLI through `@anthropic-ai/claude-code`
- `auto`: choose OpenAI/Codex first when OpenAI credentials exist, otherwise
  Anthropic when Anthropic credentials exist

Recommended GitHub secrets or variables:

- `RATO_AGENT_PROVIDER` (defaults to `anthropic` in GitHub Actions)
- `ANTHROPIC_AUTH_TOKEN`
- `RATO_CLAUDE_AUTH_TOKEN`
- `ANTHROPIC_API_KEY`
- `OPENROUTER_API_KEY`
- `RATO_AGENT_MODEL`
- `RATO_AGENT_FAST_MODEL`
- `RATO_AGENT_REVIEW_MODEL`
- `RATO_EMBEDDING_MODEL`
- `RATO_AUDIO_MODEL`
- `RATO_GH_TOKEN`
- `RATO_AUTONOMY` GitHub variable

Defaults:

- Primary model: `gpt-5.4-mini`
- Fast model: `gpt-5-mini`
- Review model: `gpt-5.1`
- Embedding model: `text-embedding-3-small`
- Audio model: `whisper-1`

## Local Fallback Tooling

The local supervisor and standalone dashboard still exist for operator fallback,
inspection, or local smoke tests:

```powershell
pwsh ./scripts/autonomy/run-local-autonomy.ps1
node ./scripts/autonomy/dashboard-server.mjs
```

Open:

```text
http://127.0.0.1:19774
```