# RATO Autonomous Agent System

This directory defines how RATO is built by autonomous agents through GitHub.
GitHub is the shared state: issues are the backlog, labels are routing, pull
requests are the work log, and CI is the deterministic judge.

## Operating Mode

RATO runs in all-out AI-made mode:

- Humans may steer with prompts, issues, comments, and priority changes.
- Humans should not write project code after this system is enabled.
- Agents may create issues, split work, write code, review pull requests, fix CI,
  and merge completed work.
- Every agent action must leave a GitHub-visible trace.
- Prompt output from one agent is valid input to the next agent.

The goal is not to minimize automation. The goal is to make automation
inspectable enough that the project can move without hidden state.

## GitHub State Machine

Labels are the protocol.

| Label | Meaning |
| --- | --- |
| `ai:inbox` | New or discovered work that has not been classified. |
| `ai:ready` | Work is ready for an implementation agent. |
| `ai:working` | An agent has claimed the issue or PR. |
| `ai:review` | A PR needs AI review or is currently being reviewed. |
| `ai:fix` | A PR needs implementation fixes after review or CI. |
| `ai:merge` | A PR is eligible for the merger agent. |
| `ai:blocked` | The agent could not make progress without a new decision or dependency. |
| `risk:r0` | Docs, metadata, tests, or isolated generated assets. |
| `risk:r1` | Small implementation change with bounded blast radius. |
| `risk:r2` | Cross-module behavior, persistence, IPC, policy, or security-sensitive logic. |
| `risk:r3` | Credentials, release, destructive migration, broad architecture, or automation permissions. |
| `type:bug` | Bug fix. |
| `type:feature` | User-visible feature. |
| `type:test` | Test-only work. |
| `type:docs` | Documentation. |
| `type:refactor` | Internal structure change. |
| `type:chore` | Tooling, CI, packaging, or maintenance. |

Agents may widen scope by creating follow-up issues, not by silently expanding a
pull request.

## Roles

- `cartographer`: scans the repo and files evidence-backed work.
- `manager`: classifies, decomposes, and routes issues.
- `worker`: implements one issue per branch and PR.
- `reviewer`: reviews PRs against the issue, architecture, and tests.
- `merger`: merges PRs that meet the merge policy and closes the loop.
- `orchestrator`: coordinates the other roles when a single agent runner is used.

Role prompts live in `docs/agents/roles/`.

## Prompt Chaining

Agents communicate by leaving structured blocks in issues and pull requests:

```markdown
## Agent Brief

Role: manager
Target: #123
Risk: risk:r1
Type: type:feature
Recommended next agent: worker

Context:
- ...

Acceptance:
- ...

Notes for next agent:
- ...
```

The next agent must read the prior blocks before acting. This is the project
memory layer.

## Merge Policy

The merger agent may merge a PR when all of these are true:

- The PR links exactly one primary issue.
- CI is green or the only failures are explicitly documented as unrelated and
  tracked in new issues.
- The reviewer agent has left a passing `Agent Review` block.
- The implementation agent has left an `Agent Work Log` block.
- The diff does not include secrets, generated dependency churn, or unrelated
  broad rewrites.
- The PR has `ai:merge` and does not have `ai:blocked`.

For `risk:r2` and `risk:r3`, the merger agent must require two separate AI
review passes in the PR thread. No human review is required by policy, but human
comments override agent decisions when present.

## Human Prompt Budget

Human input should appear as high-level steering, not code:

- Open or edit goal issues.
- Comment on priorities.
- Reject a direction by commenting on the issue or PR.
- Change labels when the routing is wrong.

If a human writes code, the PR should disclose it in the `Agent Work Log`.
