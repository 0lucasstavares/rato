[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("cartographer", "manager", "worker", "reviewer", "merger", "orchestrator")]
    [string]$Role,

    [switch]$PrintOnly
)

$ErrorActionPreference = "Stop"

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..")
Set-Location $repoRoot

function Read-RepoFile($RelativePath) {
    $path = Join-Path $repoRoot $RelativePath
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Missing required file: $RelativePath"
    }
    Get-Content -Raw -Encoding UTF8 -LiteralPath $path
}

function Try-CommandText($ScriptBlock) {
    try {
        $output = & $ScriptBlock 2>&1
        if ($LASTEXITCODE -ne 0) {
            return "Command failed:`n$($output -join "`n")"
        }
        return ($output -join "`n")
    }
    catch {
        return "Command unavailable: $($_.Exception.Message)"
    }
}

function Get-ConfiguredHarnessSummary {
    $providers = @()
    if ($env:OPENAI_API_KEY) {
        $providers += "OPENAI_API_KEY configured"
    }
    if ($env:CHATGPT_API_KEY) {
        $providers += "CHATGPT_API_KEY configured"
    }
    if ($env:ANTHROPIC_API_KEY) {
        $providers += "ANTHROPIC_API_KEY configured"
    }
    if ($env:OPENROUTER_API_KEY) {
        $providers += "OPENROUTER_API_KEY configured"
    }
    if (-not $providers) {
        $providers += "No provider API keys detected in environment"
    }

    $preferred = $env:RATO_AGENT_HARNESS
    if (-not $preferred) {
        $preferred = $env:RATO_AGENT_PROVIDER
    }
    if (-not $preferred) {
        $preferred = "auto"
    }

    $model = $env:RATO_AGENT_MODEL
    if (-not $model) {
        $model = "gpt-5.1-codex-max"
    }
    $fastModel = $env:RATO_AGENT_FAST_MODEL
    if (-not $fastModel) {
        $fastModel = "gpt-5-mini"
    }
    $reviewModel = $env:RATO_AGENT_REVIEW_MODEL
    if (-not $reviewModel) {
        $reviewModel = "gpt-5.1"
    }
    $embeddingModel = $env:RATO_EMBEDDING_MODEL
    if (-not $embeddingModel) {
        $embeddingModel = "text-embedding-3-small"
    }
    $audioModel = $env:RATO_AUDIO_MODEL
    if (-not $audioModel) {
        $audioModel = "whisper-1"
    }

    $models = @(
        "Primary model: $model",
        "Fast model: $fastModel",
        "Review model: $reviewModel",
        "Embedding model: $embeddingModel",
        "Audio model: $audioModel"
    )

    return "Preferred harness: $preferred`n" + ($providers -join "`n") + "`n" + ($models -join "`n")
}

function Get-FirstCommandToken($CommandLine) {
    $parseErrors = $null
    $tokens = [System.Management.Automation.PSParser]::Tokenize($CommandLine, [ref]$parseErrors)
    $token = $tokens | Where-Object { $_.Type -in @("Command", "String") } | Select-Object -First 1
    if (-not $token) {
        return $null
    }
    return $token.Content.Trim("'`"")
}

function Split-CommandLine($CommandLine) {
    $parseErrors = $null
    $tokens = [System.Management.Automation.PSParser]::Tokenize($CommandLine, [ref]$parseErrors)
    if ($parseErrors) {
        throw "Failed to parse RATO_AGENT_COMMAND '$CommandLine'."
    }
    $commandTokens = @($tokens | Where-Object {
        $_.Type -in @("Command", "CommandArgument", "String", "Number")
    })
    if (-not $commandTokens) {
        throw "RATO_AGENT_COMMAND is empty."
    }

    return @{
        Command = $commandTokens[0].Content.Trim("'`"")
        Arguments = @($commandTokens | Select-Object -Skip 1 | ForEach-Object { $_.Content.Trim("'`"") })
    }
}

function Invoke-AgentCommand($CommandLine, $Prompt) {
    $parsedCommand = Split-CommandLine $CommandLine
    $commandName = $parsedCommand.Command
    if (-not $commandName -or -not (Get-Command $commandName -ErrorAction SilentlyContinue)) {
        Write-Warning "RATO_AGENT_COMMAND '$CommandLine' was not found on PATH; printing prompt only."
        Write-Output $Prompt
        return 0
    }

    $Prompt | & $commandName @($parsedCommand.Arguments)
    return $LASTEXITCODE
}

function Invoke-Checked($Exe, [string[]]$Arguments) {
    Write-Host "Running: $Exe $($Arguments -join ' ')"
    & $Exe @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "Command failed with exit code $LASTEXITCODE`: $Exe $($Arguments -join ' ')"
    }
}

function Get-SafeRefSegment($Value) {
    if (-not $Value) {
        return "local"
    }
    $safe = $Value.ToLowerInvariant() -replace "[^a-z0-9._-]+", "-"
    $safe = $safe.Trim("-._")
    if (-not $safe) {
        return "local"
    }
    return $safe
}

function Write-AgentSummary {
    param(
        [string[]]$Lines
    )

    if (-not $env:GITHUB_STEP_SUMMARY) {
        return
    }

    Add-Content -LiteralPath $env:GITHUB_STEP_SUMMARY -Value (($Lines -join "`n") + "`n")
}
function Assert-RoleDidNotEditRepository {
    if ($Role -eq "worker") {
        return
    }
    if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
        return
    }

    $status = @(git status --porcelain)
    if (-not $status -or $status.Count -eq 0) {
        return
    }

    Write-AgentSummary @(
        "## Agent Outcome",
        "",
        "- Role: $Role",
        "- Outcome: unauthorized repository edits",
        "- Detail: Non-worker roles are not allowed to modify repository files."
    )

    $formattedStatus = $status -join "; "
    throw "Role '$Role' modified repository files unexpectedly: $formattedStatus"
}


function Publish-WorkerChanges {
    if ($Role -ne "worker") {
        return @{
            Outcome = "not-worker"
            Detail = "Role is not worker."
            RequeuedManager = $false
        }
    }
    if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
        throw "Worker changed files cannot be published because git is unavailable."
    }
    if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
        throw "Worker changed files cannot be published because gh is unavailable."
    }

    $status = @(git status --porcelain)
    if (-not $status -or $status.Count -eq 0) {
        Write-Host "Worker produced no repository diff; no PR to open."
        $detail = "Worker produced no repository diff; no PR was opened."
        Write-AgentSummary @(
            "## Agent Outcome",
            "",
            "- Role: worker",
            "- Outcome: no repository diff",
            "- Harness: $($env:RATO_AGENT_HARNESS)",
            "- Detail: $detail"
        )
        return @{
            Outcome = "no-diff"
            Detail = $detail
            RequeuedManager = $false
        }
    }

    Write-Host "Worker produced repository changes:"
    $status | ForEach-Object { Write-Host $_ }

    $harness = Get-SafeRefSegment $env:RATO_AGENT_HARNESS
    if ($harness -eq "local") {
        $harness = Get-SafeRefSegment $env:RATO_AGENT_PROVIDER
    }
    if ($harness -eq "local" -or $harness -eq "auto") {
        $harness = Get-SafeRefSegment $env:RATO_AGENT_ID
    }
    if ($harness -eq "local") {
        $harness = "agent"
    }
    $runId = Get-SafeRefSegment $env:GITHUB_RUN_ID
    $runAttempt = Get-SafeRefSegment $env:GITHUB_RUN_ATTEMPT
    $branch = "ai/worker/$harness-$runId-$runAttempt"
    if ($runId -eq "local") {
        $branch = "ai/worker/$harness-$(Get-Date -Format 'yyyyMMdd-HHmmss')"
    }

    Invoke-Checked "git" @("config", "user.name", "rato-agent")
    Invoke-Checked "git" @("config", "user.email", "rato-agent@users.noreply.github.com")
    Invoke-Checked "git" @("switch", "-c", $branch)
    Invoke-Checked "git" @("add", "-A", "--", ".")

    $staged = @(git diff --cached --name-only)
    if (-not $staged -or $staged.Count -eq 0) {
        Write-Host "Worker diff disappeared after staging filters; no PR to open."
        $detail = "Worker changes disappeared after staging filters; no PR was opened."
        Write-AgentSummary @(
            "## Agent Outcome",
            "",
            "- Role: worker",
            "- Outcome: empty staged diff",
            "- Harness: $($env:RATO_AGENT_HARNESS)",
            "- Detail: $detail"
        )
        return @{
            Outcome = "empty-staged-diff"
            Detail = $detail
            RequeuedManager = $false
        }
    }

    $title = "Autonomous $harness worker run $runId"
    $commitMessage = "feat(agent): autonomous $harness worker changes ($runId)"
    Invoke-Checked "git" @("commit", "-m", $commitMessage)
    Invoke-Checked "git" @("push", "--set-upstream", "origin", $branch)

    $body = @"
## Agent Assessment

Autonomous worker run published repository changes from GitHub Actions run `$runId`.

## Agent Verification

The worker harness committed the resulting diff and opened this PR automatically. CI and reviewer workflows should now inspect the branch.

## Agent Notes

- Branch: `$branch`
- Harness: `$harness`
- Run: $env:GITHUB_SERVER_URL/$env:GITHUB_REPOSITORY/actions/runs/$env:GITHUB_RUN_ID
"@

    $existing = @(gh pr list --head $branch --state open --json number --jq ".[].number")
    if ($existing -and $existing.Count -gt 0) {
        Write-Host "PR already exists for ${branch}: #$($existing[0])"
        $detail = "PR already exists for branch ${branch}: #$($existing[0])."
        Write-AgentSummary @(
            "## Agent Outcome",
            "",
            "- Role: worker",
            "- Outcome: existing PR reused",
            "- Harness: $($env:RATO_AGENT_HARNESS)",
            "- Detail: $detail"
        )
        return @{
            Outcome = "existing-pr"
            Detail = $detail
            RequeuedManager = $false
        }
    }

    Invoke-Checked "gh" @(
        "pr",
        "create",
        "--title",
        $title,
        "--body",
        $body,
        "--base",
        "main",
        "--head",
        $branch
    )
    $detail = "Worker published branch $branch and opened a pull request."
    Write-AgentSummary @(
        "## Agent Outcome",
        "",
        "- Role: worker",
        "- Outcome: pull request opened",
        "- Harness: $($env:RATO_AGENT_HARNESS)",
        "- Branch: $branch",
        "- Detail: $detail"
    )
    return @{
        Outcome = "opened-pr"
        Detail = $detail
        RequeuedManager = $false
    }
}

function Test-CheckRollupGreen($Rollup) {
    $checks = @($Rollup)
    if (-not $checks -or $checks.Count -eq 0) {
        return $false
    }

    foreach ($check in $checks) {
        if ($check.status -ne "COMPLETED") {
            return $false
        }
        if ($check.conclusion -and $check.conclusion -ne "SUCCESS" -and $check.conclusion -ne "SKIPPED") {
            return $false
        }
    }

    return $true
}

function Test-HasSuccessfulWorkflow($Rollup, $WorkflowName) {
    foreach ($check in @($Rollup)) {
        if ($check.workflowName -eq $WorkflowName -and $check.status -eq "COMPLETED" -and $check.conclusion -eq "SUCCESS") {
            return $true
        }
    }
    return $false
}

function Test-HasLabel($Pr, $Name) {
    foreach ($label in @($Pr.labels)) {
        if ($label.name -eq $Name) {
            return $true
        }
    }
    return $false
}

function Merge-EligiblePullRequests {
    if ($Role -ne "merger") {
        return
    }
    if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
        throw "Merger cannot inspect or merge pull requests because gh is unavailable."
    }

    $json = gh pr list --state open --limit 30 --json number,title,isDraft,mergeStateStatus,labels,statusCheckRollup,url
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to list pull requests for merger."
    }
    $prs = @($json | ConvertFrom-Json)
    if (-not $prs -or $prs.Count -eq 0) {
        Write-Host "No open pull requests to merge."
        return
    }

    foreach ($pr in $prs) {
        $number = [string]$pr.number
        $reasons = @()
        if ($pr.isDraft) {
            $reasons += "draft"
        }
        if ($pr.mergeStateStatus -ne "CLEAN" -and $pr.mergeStateStatus -ne "HAS_HOOKS") {
            $reasons += "merge state $($pr.mergeStateStatus)"
        }
        if (Test-HasLabel $pr "ai:blocked") {
            $reasons += "ai:blocked"
        }
        if (-not (Test-CheckRollupGreen $pr.statusCheckRollup)) {
            $reasons += "checks not green"
        }
        if (-not (Test-HasSuccessfulWorkflow $pr.statusCheckRollup "agent-reviewer")) {
            $reasons += "agent-reviewer not green"
        }

        if ($reasons.Count -gt 0) {
            Write-Host "PR #$number not eligible: $($reasons -join ', ')"
            continue
        }

        $body = @"
## Agent Merge Decision

Verdict: merged
Reason:
- Autonomous merge policy accepted this PR.
- Merge state is clean.
- CI and agent-reviewer checks are green.
- PR is not draft and not blocked.

Checks:
- CI: green
- Review count: agent-reviewer check passed
- Linked issue: see PR body
- Risk: enforced by checks, not human gate
"@
        Invoke-Checked "gh" @("pr", "comment", $number, "--body", $body)
        Invoke-Checked "gh" @("pr", "merge", $number, "--squash", "--delete-branch", "--admin")
    }
}

$constitution = Read-RepoFile "docs\agents\CONSTITUTION.md"
$overview = Read-RepoFile "docs\agents\README.md"
$rolePrompt = Read-RepoFile "docs\agents\roles\$Role.md"
$rootReadme = Read-RepoFile "README.md"
$providerSummary = Get-ConfiguredHarnessSummary
$agentIdentity = $env:RATO_AGENT_ID
if (-not $agentIdentity) {
    $agentIdentity = $env:RATO_AGENT_HARNESS
}
if (-not $agentIdentity) {
    $agentIdentity = $env:RATO_AGENT_PROVIDER
}
if (-not $agentIdentity) {
    $agentIdentity = "auto"
}

$gitState = Try-CommandText { git status --short --branch }
$recentCommits = Try-CommandText { git log --oneline -20 }

$issues = "gh unavailable"
$prs = "gh unavailable"
$labels = "gh unavailable"
if (Get-Command gh -ErrorAction SilentlyContinue) {
    $issues = Try-CommandText { gh issue list --state open --limit 80 --json number,title,labels,assignees,updatedAt,url }
    $prs = Try-CommandText { gh pr list --state open --limit 50 --json number,title,labels,headRefName,updatedAt,url,statusCheckRollup }
    $labels = Try-CommandText { gh label list --limit 80 --json name,description }
}

$prompt = @"
# RATO Autonomous Agent Invocation

You are running as role: $Role
Agent identity: $agentIdentity

Act on the repository directly. Use GitHub issues and pull requests as the
control plane. Do not return a passive plan if you can take the next concrete
action. Leave structured comments using the headings defined in the constitution.

## Constitution

$constitution

## System Overview

$overview

## Role Prompt

$rolePrompt

## Project README

$rootReadme

## Harness Environment

```text
$providerSummary
```

## Current Git State

```text
$gitState
```

## Recent Commits

```text
$recentCommits
```

## Open GitHub Issues

```json
$issues
```

## Open GitHub Pull Requests

```json
$prs
```

## Available GitHub Labels

```json
$labels
```

## Issue Creation Protocol

Agents are allowed and expected to create GitHub issues when they discover work.
Before creating an issue, quickly check the open issue list for an obvious
duplicate. If no duplicate exists, create the issue immediately instead of
leaving the work only in a comment, PR body, TODO, or final summary.

Use this shape:

```markdown
## Agent Brief

Role: <creating role>
Target: new issue
Risk: risk:r?
Type: type:?
Recommended next agent: manager | worker | reviewer

Context:
- ...

Acceptance:
- ...

Likely Files:
- ...

Verification:
- ...

Evidence:
- ...
```

Create issues with labels at creation time where possible:

```text
gh issue create --title "AI discovered: <short imperative title>" --body-file <body.md> --label ai:inbox --label risk:r1 --label type:feature
```

Use `ai:ready` instead of `ai:inbox` only when the issue already has enough
context and acceptance criteria for a worker to implement it.

## Required Behavior

- Prefer taking one complete action over producing advice.
- If acting on code, create or use a branch and open/update a PR.
- If acting on issues, create missing issues, update labels, and leave the
  required agent block.
- If discovering follow-up work, create GitHub issues and reference their issue
  numbers in the current issue, PR, or review.
- If blocked, leave an `Agent Blocker` comment with the exact missing condition.
- Keep all progress visible on GitHub.
"@

if ($PrintOnly -or -not $env:RATO_AGENT_COMMAND) {
    if (-not $env:RATO_AGENT_COMMAND) {
        Write-Host "RATO_AGENT_COMMAND is not set; printing prompt only."
    }
    Write-Output $prompt
    exit 0
}

Merge-EligiblePullRequests

$agentExitCode = Invoke-AgentCommand $env:RATO_AGENT_COMMAND $prompt
if ($agentExitCode -ne 0) {
    exit $agentExitCode
}

$publishResult = Publish-WorkerChanges
if ($Role -eq "worker" -and $publishResult) {
    Write-Host "Worker outcome: $($publishResult.Outcome)"
}
exit 0