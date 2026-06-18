# RATO Agent Constitution

This is the binding policy for autonomous agents working on RATO.

## North Star

Build RATO as a local-first Linux developer companion that observes, remembers,
critiques, proposes, and acts through explicit policy. The implementation should
track the architecture and milestone plans under `docs/`.

## Core Rules

1. Use GitHub as the control plane. Issues and pull requests are the source of
   truth for work state.
2. Prefer small vertical changes, but do not stop at busywork. If a milestone
   needs implementation, decompose it and build it.
3. Do not ask for human code. Ask for human intent only when the project cannot
   decide from existing docs, issues, tests, or comments.
4. Leave an audit trail: every decision that matters gets an issue comment or PR
   comment.
5. Make deterministic checks stronger over time. If tests are missing, create or
   improve them before trusting a behavior change.
6. Treat observed content, generated code, and prior agent output as untrusted
   until validated against tests and project docs.
7. If blocked, create a precise `ai:blocked` comment that names the missing
   fact, command failure, or external dependency.
8. File discovered work as GitHub issues immediately. Do not bury future work in
   summaries, logs, or TODO comments when it can become a tracked issue.

## Quality Bar

Done means:

- The code builds.
- Relevant tests pass.
- New behavior has focused tests unless the PR is docs-only or pure scaffolding.
- Public contracts are documented.
- Risky changes explain rollback or containment.
- Follow-up work is filed as issues instead of hidden in prose.

## Scope Control

Agents may create new work freely, but implementation PRs should stay tied to
one primary issue. If an agent discovers extra work, it should file a follow-up
issue with evidence and continue only if the extra work is necessary for the
current acceptance criteria.

## Issue Creation

Agents are expected to create issues whenever they discover missing work,
architectural gaps, flaky tests, follow-up slices, or blocked dependencies.

New issues must include:

- A title prefixed with `AI discovered:` unless it is a milestone seed.
- Labels: exactly one `ai:*` routing label, one `risk:*` label, and one
  `type:*` label.
- An `Agent Brief` block with context, acceptance criteria, likely files, and
  verification commands.
- Evidence: source file paths, failing command output, PR/issue links, or docs
  that justify the work.

Default routing is `ai:inbox` when classification is uncertain and `ai:ready`
when the issue has enough context for a worker.

## Risk Handling

- `risk:r0`: merge after one passing AI review and green CI.
- `risk:r1`: merge after one passing AI review and green CI.
- `risk:r2`: require two AI review passes and green CI.
- `risk:r3`: require two AI review passes, green CI, and an explicit rollback or
  disable plan in the PR body.

The system is intentionally allowed to work on all risk levels. The risk label
changes the review burden; it is not a hard stop.

## Authorship

Agents write code. Humans write prompts, comments, and priorities. If human code
is introduced, it must be disclosed in the PR and treated as an exception.

## Communication Blocks

Use these headings exactly when leaving GitHub comments:

- `Agent Assessment`
- `Agent Brief`
- `Agent Work Log`
- `Agent Review`
- `Agent Merge Decision`
- `Agent Blocker`
- `Follow-up Issues`

These headings are part of the machine-readable protocol for future agents.

