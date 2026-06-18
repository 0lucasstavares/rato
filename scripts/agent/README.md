# Agent Scripts

These scripts are thin wrappers around GitHub and an external agent command.
They do not contain the intelligence. They assemble project context, role
prompts, and GitHub state so an agent can act consistently.

## Bootstrap GitHub

```powershell
powershell -ExecutionPolicy Bypass -File ./scripts/agent/bootstrap-github.ps1
```

This creates the label vocabulary used by the autonomous loops. Add
`-SeedMilestoneIssues` to create initial issues from the milestone plan files.

Local bootstrap uses the local GitHub CLI auth context. If the repository secret
is already configured but local `gh auth status` still fails, refresh or export a
token in the same shell before running the script:

```powershell
gh auth refresh -h github.com -s repo -s workflow
# or
$env:GH_TOKEN = "<fine-grained-token-for-this-repo>"
```

The GitHub Actions workflows expect these repository secrets:

- `RATO_AGENT_COMMAND`: public workflow config. The repo defaults this to
  `pwsh ./scripts/agent/run-provider-agent.ps1`.
- `RATO_GH_TOKEN`: optional fine-grained token. If omitted, workflows fall back
  to `github.token`.
- `RATO_AGENT_PROVIDER`: optional provider preference: `openai`, `anthropic`,
  `openrouter`, or `auto`.
- `OPENAI_API_KEY`: OpenAI API key for Codex/OpenAI-compatible wrappers.
- `CHATGPT_API_KEY`: optional alias for wrappers that expect "ChatGPT" naming.
- `ANTHROPIC_API_KEY`: Anthropic API key for Claude/Anthropic wrappers.
- `OPENROUTER_API_KEY`: optional OpenRouter key.
- `RATO_AGENT_MODEL`: primary coding model. Default: `gpt-5.1-codex-max`.
- `RATO_AGENT_FAST_MODEL`: cheaper triage/planning model. Default:
  `gpt-5-mini`.
- `RATO_AGENT_REVIEW_MODEL`: review/checker model. Default: `gpt-5.1`.
- `RATO_EMBEDDING_MODEL`: embedding model. Default:
  `text-embedding-3-small`.
- `RATO_AUDIO_MODEL`: audio/transcription model. Default: `whisper-1`.

Known available OpenAI models for this setup:

```text
gpt-5.2
gpt-4o
gpt-4
gpt-4o-mini
o4-mini
gpt-5.4-pro
gpt-5.5-pro
whisper-1
gpt-audio-2025-08-28
gpt-realtime-whisper
gpt-5.1-codex-max
text-embedding-3-small
gpt-5.1
gpt-5-mini
```

## Run One Role

```powershell
powershell -ExecutionPolicy Bypass -File ./scripts/agent/run-agent-role.ps1 -Role manager
```

By default, the script prints the full prompt unless `RATO_AGENT_COMMAND` is set.
The GitHub workflows set it to the repo-owned provider wrapper:
`pwsh ./scripts/agent/run-provider-agent.ps1`.

Example shape:

```powershell
$env:RATO_AGENT_COMMAND = "rato-agent-wrapper"
powershell -ExecutionPolicy Bypass -File ./scripts/agent/run-agent-role.ps1 -Role worker
```

The provider wrapper chooses Anthropic first when `ANTHROPIC_API_KEY` is
available, otherwise OpenAI when `OPENAI_API_KEY` or `CHATGPT_API_KEY` is
available.
