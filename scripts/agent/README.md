# Agent Scripts

These scripts are thin wrappers around GitHub and an external agent command.
They do not contain the intelligence. They assemble project context, role
prompts, and GitHub state so an agent can act consistently.

The live workflow is documented in `docs/agents/AUTONOMOUS_WORKFLOW.md`.

## Bootstrap GitHub

```powershell
powershell -ExecutionPolicy Bypass -File ./scripts/agent/bootstrap-github.ps1
```

This creates the label vocabulary used by the autonomous loop. Add
`-SeedMilestoneIssues` to create initial issues from the milestone plan files.

Local bootstrap uses the local GitHub CLI auth context. If `gh auth status`
fails, refresh auth or export a token in the same shell before running the
bootstrap:

```powershell
gh auth refresh -h github.com -s repo -s workflow
# or
$env:GH_TOKEN = "<fine-grained-token-for-this-repo>"
```

## Run One Role

```powershell
powershell -ExecutionPolicy Bypass -File ./scripts/agent/run-agent-role.ps1 -Role manager
```

By default, the script prints the full prompt unless `RATO_AGENT_COMMAND` is
set. The GitHub Actions workflows default it to the repo-owned provider wrapper:
`pwsh ./scripts/agent/run-provider-agent.ps1`.

Supported roles:

- `scrum-master`
- `manager`
- `worker`
- `reviewer`
- `merger`
- `cartographer`
- `orchestrator`

The provider wrapper supports `auto` mode, which chooses OpenAI/Codex first
when OpenAI credentials exist, otherwise Anthropic when Anthropic credentials
exist. Set `RATO_AGENT_PROVIDER=anthropic` to force Claude Code.

## GitHub Actions autonomy

Primary workflows:

```text
.github/workflows/agent-scrum-master.yml
.github/workflows/agent-manager.yml
.github/workflows/agent-worker.yml
.github/workflows/agent-reviewer.yml
.github/workflows/agent-merger.yml
```

Control workflows:

```text
.github/workflows/autonomy-on.yml
.github/workflows/autonomy-off.yml
```

## Local fallback

The local supervisor still exists for fallback testing:

```powershell
pwsh ./scripts/autonomy/run-local-autonomy.ps1
pwsh ./scripts/autonomy/run-local-autonomy.ps1 -Once
```

The standalone dashboard still exists for local inspection:

```powershell
node ./scripts/autonomy/dashboard-server.mjs
```

Open:

```text
http://127.0.0.1:19774
```