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

function Invoke-Logged($Exe, [string[]]$Arguments) {
    Write-Host "Running provider command: $Exe $($Arguments -join ' ')"
    & $Exe @Arguments
    exit $LASTEXITCODE
}

$provider = $env:RATO_AGENT_PROVIDER
if (-not $provider -or $provider -eq "auto") {
    if ($env:ANTHROPIC_API_KEY) {
        $provider = "anthropic"
    }
    elseif ($env:OPENAI_API_KEY -or $env:CHATGPT_API_KEY) {
        $provider = "openai"
    }
    else {
        throw "No provider API key configured. Set ANTHROPIC_API_KEY or OPENAI_API_KEY."
    }
}

switch ($provider.ToLowerInvariant()) {
    "anthropic" {
        if (-not $env:ANTHROPIC_API_KEY) {
            throw "RATO_AGENT_PROVIDER=anthropic but ANTHROPIC_API_KEY is not configured."
        }
        Require-Command "npx"
        Invoke-Logged "npx" @(
            "-y",
            "@anthropic-ai/claude-code",
            "-p",
            $prompt,
            "--dangerously-skip-permissions"
        )
    }
    "openai" {
        if (-not $env:OPENAI_API_KEY -and $env:CHATGPT_API_KEY) {
            $env:OPENAI_API_KEY = $env:CHATGPT_API_KEY
        }
        if (-not $env:OPENAI_API_KEY) {
            throw "RATO_AGENT_PROVIDER=openai but OPENAI_API_KEY or CHATGPT_API_KEY is not configured."
        }
        $model = $env:RATO_AGENT_MODEL
        if (-not $model) {
            $model = "gpt-5.1-codex-max"
        }
        Require-Command "npx"
        Invoke-Logged "npx" @("-y", "@openai/codex", "exec", "--model", $model, $prompt)
    }
    default {
        throw "Unsupported RATO_AGENT_PROVIDER '$provider'. Use auto, anthropic, or openai."
    }
}
