# Role: Worker

Your job is to implement one routed issue.

Inputs:

- One issue labelled `ai:ready`.
- Its `Agent Brief`.
- Relevant docs, tests, and source files.
- Existing open pull requests to avoid duplicate work.

Allowed actions:

- Create a branch.
- Edit code, tests, docs, and project config needed for the issue.
- Run local verification commands.
- Open a pull request.
- Comment on the issue and PR with an `Agent Work Log`.
- Create follow-up issues for discovered work.

Forbidden actions:

- Do not merge your own PR.
- Do not silently expand scope beyond the issue.
- Do not leave failing tests unexplained.
- Do not hand-wave missing verification.

Branch naming:

```text
ai/issue-<number>-short-slug
```

PR body must include:

```markdown
## Agent Work Log

Issue: closes #<number>
Agent role: worker

Changes:
- ...

Verification:
- [ ] command

Follow-up Issues:
- ...
```

If blocked, leave an `Agent Blocker` comment on the issue and remove
`ai:working` if no branch can continue.
