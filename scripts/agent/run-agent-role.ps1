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

function Get-ConfiguredProviderSummary {
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

    $preferred = $env:RATO_AGENT_PROVIDER
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

    return "Preferred provider: $preferred`n" + ($providers -join "`n") + "`n" + ($models -join "`n")
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

function Invoke-AgentCommand($CommandLine, $Prompt) {
    $commandName = Get-FirstCommandToken $CommandLine
    if (-not $commandName -or -not (Get-Command $commandName -ErrorAction SilentlyContinue)) {
        Write-Warning "RATO_AGENT_COMMAND '$CommandLine' was not found on PATH; printing prompt only."
        Write-Output $Prompt
        return 0
    }

    $psi = [System.Diagnostics.ProcessStartInfo]::new()
    if ($IsWindows -or $env:OS -eq "Windows_NT") {
        $psi.FileName = "powershell.exe"
        $escaped = $CommandLine.Replace('"', '\"')
        $psi.Arguments = "-NoProfile -ExecutionPolicy Bypass -Command `"$escaped`""
    }
    else {
        $psi.FileName = "/bin/bash"
        $escaped = $CommandLine.Replace("'", "'\''")
        $psi.Arguments = "-lc '$escaped'"
    }
    $psi.RedirectStandardInput = $true
    $psi.RedirectStandardOutput = $false
    $psi.RedirectStandardError = $false
    $psi.UseShellExecute = $false

    $process = [System.Diagnostics.Process]::Start($psi)
    $process.StandardInput.Write($Prompt)
    $process.StandardInput.Close()
    $process.WaitForExit()
    return $process.ExitCode
}

$constitution = Read-RepoFile "docs\agents\CONSTITUTION.md"
$overview = Read-RepoFile "docs\agents\README.md"
$rolePrompt = Read-RepoFile "docs\agents\roles\$Role.md"
$rootReadme = Read-RepoFile "README.md"
$providerSummary = Get-ConfiguredProviderSummary

$gitState = Try-CommandText { git status --short --branch }
$recentCommits = Try-CommandText { git log --oneline -20 }

$issues = "gh unavailable"
$prs = "gh unavailable"
if (Get-Command gh -ErrorAction SilentlyContinue) {
    $issues = Try-CommandText { gh issue list --state open --limit 80 --json number,title,labels,assignees,updatedAt,url }
    $prs = Try-CommandText { gh pr list --state open --limit 50 --json number,title,labels,headRefName,updatedAt,url,statusCheckRollup }
}

$prompt = @"
# RATO Autonomous Agent Invocation

You are running as role: $Role

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

## Provider Environment

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

## Required Behavior

- Prefer taking one complete action over producing advice.
- If acting on code, create or use a branch and open/update a PR.
- If acting on issues, update labels and leave the required agent block.
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

exit (Invoke-AgentCommand $env:RATO_AGENT_COMMAND $prompt)
