[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"

$prompt = [Console]::In.ReadToEnd()
if (-not $prompt) {
    throw "No prompt received on stdin."
}

function Require-Command($Name) {
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "Required command '$Name' was not found on PATH."
    }
}

function Invoke-Logged($Harness, $Exe, [string[]]$Arguments) {
    Write-Host "Running agent harness '$Harness': $Exe $($Arguments -join ' ')"
    & $Exe @Arguments
    exit $LASTEXITCODE
}

$harness = $env:RATO_AGENT_HARNESS
if (-not $harness) {
    $harness = $env:RATO_AGENT_PROVIDER
}
if (-not $harness -or $harness -eq "auto") {
    if ($env:OPENAI_API_KEY -or $env:CHATGPT_API_KEY) {
        $harness = "codex"
    }
    elseif ($env:ANTHROPIC_API_KEY) {
        $harness = "claude-code"
    }
    else {
        throw "No provider API key configured. Set ANTHROPIC_API_KEY or OPENAI_API_KEY."
    }
}

$normalizedHarness = $harness.ToLowerInvariant()
switch ($normalizedHarness) {
    { $_ -in @("claude-code", "claude", "anthropic") } {
        if (-not $env:ANTHROPIC_API_KEY) {
            throw "Claude Code harness requested but ANTHROPIC_API_KEY is not configured."
        }
        Require-Command "npx"
        Invoke-Logged "claude-code" "npx" @(
            "-y",
            "@anthropic-ai/claude-code",
            "-p",
            $prompt,
            "--dangerously-skip-permissions"
        )
    }
    { $_ -in @("codex", "openai") } {
        if (-not $env:OPENAI_API_KEY -and $env:CHATGPT_API_KEY) {
            $env:OPENAI_API_KEY = $env:CHATGPT_API_KEY
        }
        if (-not $env:OPENAI_API_KEY) {
            throw "Codex harness requested but OPENAI_API_KEY or CHATGPT_API_KEY is not configured."
        }
        $env:CODEX_API_KEY = $env:OPENAI_API_KEY
        $env:OPENAI_KEY = $env:OPENAI_API_KEY
        $env:OPENAI_API_TOKEN = $env:OPENAI_API_KEY
        $model = $env:RATO_AGENT_MODEL
        if (-not $model) {
            $model = "gpt-5.1-codex-max"
        }
        Require-Command "npx"
        Invoke-Logged "codex" "npx" @(
            "-y",
            "@openai/codex",
            "exec",
            "--model",
            $model,
            "--sandbox",
            "danger-full-access",
            $prompt
        )
    }
    default {
        throw "Unsupported RATO_AGENT_HARNESS '$harness'. Use auto, codex, or claude-code."
    }
}

