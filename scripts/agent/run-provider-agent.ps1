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

function Get-CommandName {
    param([string[]]$Candidates)
    foreach ($candidate in $Candidates) {
        if (Get-Command $candidate -ErrorAction SilentlyContinue) {
            return $candidate
        }
    }
    return $null
}

function Get-CodexCommand {
    $commandName = Get-CommandName @("codex", "codex.exe", "codex.cmd")
    if ($commandName) {
        return $commandName
    }

    $whereResult = @(where.exe codex 2>$null)
    if ($LASTEXITCODE -eq 0 -and $whereResult.Count -gt 0) {
        return $whereResult[0].Trim()
    }

    $candidateRoots = @(
        $env:LOCALAPPDATA,
        (Join-Path $env:USERPROFILE 'AppData\Local')
    ) | Where-Object { $_ }

    foreach ($root in $candidateRoots) {
        $installedPath = Join-Path $root 'Programs\OpenAI\Codex\bin\codex.exe'
        if (Test-Path -LiteralPath $installedPath) {
            return $installedPath
        }
    }

    return $null
}

function Ensure-CodexCli {
    $codexCommand = Get-CodexCommand
    if ($codexCommand) {
        return $codexCommand
    }

    $npmCommand = Get-CommandName @("npm", "npm.cmd")
    if (-not $npmCommand) {
        throw "Codex CLI is unavailable and npm was not found on PATH."
    }

    Write-Host "Installing Codex CLI globally via $npmCommand"
    & $npmCommand install -g @openai/codex@latest
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to install Codex CLI with $npmCommand install -g @openai/codex@latest"
    }

    $codexCommand = Get-CodexCommand
    if ($codexCommand) {
        return $codexCommand
    }

    throw "Codex CLI is still unavailable after installation."
}

function Get-FirstNonEmptyValue {
    param([string[]]$Values)
    foreach ($value in $Values) {
        if ($value) {
            return $value
        }
    }
    return $null
}

function Test-LoopbackDenyProxy([string]$Value) {
    if (-not $Value) {
        return $false
    }
    return $Value -match '^http://127\.0\.0\.1:9/?$'
}

function Clear-CodexSandboxOverrides {
    Remove-Item Env:CODEX_SANDBOX_NETWORK_DISABLED -ErrorAction SilentlyContinue

    foreach ($name in @('HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY', 'GIT_HTTP_PROXY', 'GIT_HTTPS_PROXY')) {
        $value = [System.Environment]::GetEnvironmentVariable($name)
        if (Test-LoopbackDenyProxy $value) {
            Remove-Item "Env:$name" -ErrorAction SilentlyContinue
        }
    }
}

function Initialize-IsolatedCodexHome([string]$RepoRoot) {
    $childHome = Join-Path $RepoRoot '.rato\codex-session-home'
    New-Item -ItemType Directory -Force -Path $childHome | Out-Null
    New-Item -ItemType Directory -Force -Path (Join-Path $childHome 'log') | Out-Null

    $sourceAuth = Join-Path $env:USERPROFILE '.codex\auth.json'
    if (-not (Test-Path -LiteralPath $sourceAuth)) {
        throw "Codex login session was not found at $sourceAuth. Run 'codex login' first."
    }

    Copy-Item -LiteralPath $sourceAuth -Destination (Join-Path $childHome 'auth.json') -Force
    return $childHome
}

$harness = $env:RATO_AGENT_HARNESS
if (-not $harness) {
    $harness = $env:RATO_AGENT_PROVIDER
}
$openAiKey = Get-FirstNonEmptyValue @(
    $env:RATO_CODEX_API_KEY
    $env:RATO_OPENAI_API_KEY
    $env:OPENAI_API_KEY
    $env:CHATGPT_API_KEY
)
if (-not $harness -or $harness -eq "auto") {
    if ($openAiKey) {
        $harness = "codex"
    }
    elseif ($env:ANTHROPIC_API_KEY) {
        $harness = "claude-code"
    }
    else {
        $harness = "codex"
    }
}

$normalizedHarness = $harness.ToLowerInvariant()
switch ($normalizedHarness) {
    { $_ -in @("claude-code", "claude", "anthropic") } {
        $anthropicToken = Get-FirstNonEmptyValue @(
            $env:RATO_CLAUDE_AUTH_TOKEN
            $env:RATO_ANTHROPIC_API_KEY
            $env:ANTHROPIC_AUTH_TOKEN
            $env:ANTHROPIC_API_KEY
        )
        if (-not $anthropicToken) {
            throw "Claude Code harness requested but no Anthropic credential is configured."
        }
        $env:ANTHROPIC_AUTH_TOKEN = $anthropicToken
        $env:ANTHROPIC_API_KEY = $anthropicToken
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
        $codexCommand = Ensure-CodexCli
        $previousCodexHome = $env:CODEX_HOME
        try {
            Clear-CodexSandboxOverrides
            if ($openAiKey) {
                $env:CODEX_API_KEY = $openAiKey
                $env:OPENAI_API_KEY = $openAiKey
                $env:OPENAI_KEY = $openAiKey
                $env:OPENAI_API_TOKEN = $openAiKey
            }
            else {
                $repoRoot = (Resolve-Path '.').Path
                $env:CODEX_HOME = Initialize-IsolatedCodexHome -RepoRoot $repoRoot
                Write-Host "No OpenAI API key configured; relying on the local Codex login session in $($env:CODEX_HOME)."
            }

            $model = $env:RATO_AGENT_MODEL
            if (-not $model) {
                $model = "gpt-5.4-mini"
            }
            Invoke-Logged "codex" $codexCommand @(
                "exec",
                "--ignore-user-config",
                "--model",
                $model,
                "--sandbox",
                "danger-full-access",
                $prompt
            )
        }
        finally {
            if ($null -eq $previousCodexHome) {
                Remove-Item Env:CODEX_HOME -ErrorAction SilentlyContinue
            }
            else {
                $env:CODEX_HOME = $previousCodexHome
            }
        }
    }
    default {
        throw "Unsupported RATO_AGENT_HARNESS '$harness'. Use auto, codex, or claude-code."
    }
}