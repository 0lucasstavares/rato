[CmdletBinding()]
param(
    [switch]$SeedMilestoneIssues
)

$ErrorActionPreference = "Stop"

function Require-Command($Name) {
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "Required command '$Name' was not found on PATH."
    }
}

Require-Command gh

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..")
Set-Location $repoRoot

$auth = & gh auth status 2>&1
if ($LASTEXITCODE -ne 0) {
    $auth | Write-Host
    throw "GitHub CLI auth is not usable. Refresh it with: gh auth refresh -h github.com"
}

$labels = @(
    @{ Name = "ai:inbox"; Color = "ededed"; Description = "Unclassified AI-discovered work" },
    @{ Name = "ai:ready"; Color = "2ea44f"; Description = "Ready for an implementation agent" },
    @{ Name = "ai:working"; Color = "1f6feb"; Description = "Claimed by an agent" },
    @{ Name = "ai:review"; Color = "8250df"; Description = "Needs AI review" },
    @{ Name = "ai:fix"; Color = "fbca04"; Description = "Needs implementation fixes" },
    @{ Name = "ai:merge"; Color = "0e8a16"; Description = "Eligible for merger agent" },
    @{ Name = "ai:blocked"; Color = "b60205"; Description = "Blocked pending a decision or dependency" },
    @{ Name = "risk:r0"; Color = "c2e0c6"; Description = "Docs, tests, metadata, or isolated assets" },
    @{ Name = "risk:r1"; Color = "7ee787"; Description = "Small bounded implementation change" },
    @{ Name = "risk:r2"; Color = "ffab70"; Description = "Cross-module, persistence, IPC, policy, or security-sensitive change" },
    @{ Name = "risk:r3"; Color = "f85149"; Description = "Credentials, release, destructive migration, or broad automation permissions" },
    @{ Name = "type:bug"; Color = "d73a4a"; Description = "Bug fix" },
    @{ Name = "type:feature"; Color = "a2eeef"; Description = "User-visible feature" },
    @{ Name = "type:test"; Color = "bfdadc"; Description = "Test work" },
    @{ Name = "type:docs"; Color = "0075ca"; Description = "Documentation" },
    @{ Name = "type:refactor"; Color = "5319e7"; Description = "Internal refactor" },
    @{ Name = "type:chore"; Color = "d4c5f9"; Description = "Tooling, CI, packaging, or maintenance" }
)

foreach ($label in $labels) {
    & gh label create $label["Name"] --color $label["Color"] --description $label["Description"] --force | Out-Host
}

if ($SeedMilestoneIssues) {
    $plans = Get-ChildItem -LiteralPath (Join-Path $repoRoot "docs\superpowers\plans") -Filter "*.md" | Sort-Object Name
    $openIssuesJson = & gh issue list --state open --limit 500 --json number,title
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to list existing GitHub issues."
    }
    $openIssues = @()
    if ($openIssuesJson) {
        $openIssues = $openIssuesJson | ConvertFrom-Json
    }

    foreach ($plan in $plans) {
        $content = Get-Content -Raw -Encoding UTF8 -LiteralPath $plan.FullName
        $firstHeading = ($content -split "`n" | Where-Object { $_ -match "^#\s+" } | Select-Object -First 1)
        if (-not $firstHeading) {
            $firstHeading = $plan.BaseName
        }
        $title = "AI seed: " + ($firstHeading -replace "^#\s+", "").Trim()
        $existing = $openIssues | Where-Object { $_.title -eq $title } | Select-Object -First 1
        if ($existing) {
            Write-Host "Issue already exists for '$title' (#$($existing.number))"
            continue
        }

        $body = @"
## Source

Seeded from ``$($plan.FullName.Substring($repoRoot.Path.Length + 1))``.

## Plan

$content
"@

        $tmp = New-TemporaryFile
        try {
            Set-Content -LiteralPath $tmp.FullName -Value $body -NoNewline -Encoding UTF8
            & gh issue create --title $title --body-file $tmp.FullName --label "ai:inbox,type:feature" | Out-Host
        }
        finally {
            Remove-Item -LiteralPath $tmp.FullName -Force
        }
    }
}

Write-Host "GitHub bootstrap complete."
