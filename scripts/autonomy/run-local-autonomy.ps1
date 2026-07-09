[CmdletBinding()]
param(
    [switch]$Once,
    [int]$IntervalSeconds = 90,
    [string]$StateDir = ".rato/autonomy"
)

$ErrorActionPreference = "Stop"

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..")
Set-Location $repoRoot

$stateRoot = Join-Path $repoRoot $StateDir
$logsDir = Join-Path $stateRoot "logs"
$worktreesRoot = Join-Path $stateRoot "worktrees"
$statePath = Join-Path $stateRoot "state.json"

function Require-Command([string]$Name) {
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "Required command '$Name' was not found on PATH."
    }
}

function Ensure-StateLayout {
    New-Item -ItemType Directory -Force -Path $stateRoot | Out-Null
    New-Item -ItemType Directory -Force -Path $logsDir | Out-Null
    New-Item -ItemType Directory -Force -Path $worktreesRoot | Out-Null
}

function Get-NowMs {
    return [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
}

function Write-Utf8NoBom([string]$Path, [string]$Content) {
    [System.IO.File]::WriteAllText($Path, $Content, (New-Object System.Text.UTF8Encoding($false)))
}

function Convert-ToHashtable($Value) {
    if ($null -eq $Value) { return $null }
    if ($Value -is [System.Collections.IDictionary]) {
        $table = @{}
        foreach ($key in $Value.Keys) {
            $table[$key] = Convert-ToHashtable $Value[$key]
        }
        return $table
    }
    if ($Value -is [System.Array]) {
        return @($Value | ForEach-Object { Convert-ToHashtable $_ })
    }
    if ($Value -is [pscustomobject]) {
        $table = @{}
        foreach ($prop in $Value.PSObject.Properties) {
            $table[$prop.Name] = Convert-ToHashtable $prop.Value
        }
        return $table
    }
    return $Value
}

function Convert-JsonArray([string]$Json) {
    $parsed = $Json | ConvertFrom-Json
    if ($null -eq $parsed) { return @() }
    if ($parsed -is [System.Array]) { return $parsed }
    return @($parsed)
}

function Read-State {
    if (-not (Test-Path -LiteralPath $statePath)) {
        return $null
    }
    return Convert-ToHashtable (Get-Content -Raw -Encoding UTF8 -LiteralPath $statePath | ConvertFrom-Json)
}

function New-WorkflowMap {
    return @{
        'scrum-master' = @{
            role = "Scrum Master"
            workflow = "local-scrum-master"
            status = "idle"
            trigger = "backlog or PR organization needed"
            cadence = "queue-driven"
            last_run_ms = $null
            last_result = "waiting for local run"
            next_action = "organize the queue and pick the next role"
            handoff = "manager | worker | reviewer | merger"
        }
        manager = @{
            role = "Manager"
            workflow = "local-manager"
            status = "idle"
            trigger = "organized backlog needs routing"
            cadence = "queue-driven"
            last_run_ms = $null
            last_result = "waiting for local run"
            next_action = "triage and create ready work"
            handoff = "worker"
        }
        worker = @{
            role = "Worker"
            workflow = "local-worker"
            status = "idle"
            trigger = "ready issue without open PR"
            cadence = "queue-driven"
            last_run_ms = $null
            last_result = "waiting for local run"
            next_action = "implement a ready issue and open a PR"
            handoff = "reviewer"
        }
        reviewer = @{
            role = "Reviewer"
            workflow = "local-reviewer"
            status = "idle"
            trigger = "open PR"
            cadence = "queue-driven"
            last_run_ms = $null
            last_result = "waiting for local run"
            next_action = "review open PRs"
            handoff = "worker | merger"
        }
        merger = @{
            role = "Merger"
            workflow = "local-merger"
            status = "idle"
            trigger = "reviewed green PR"
            cadence = "queue-driven"
            last_run_ms = $null
            last_result = "waiting for local run"
            next_action = "merge eligible PRs"
            handoff = "scrum-master"
        }
    }
}

function New-State {
    $workflows = New-WorkflowMap
    return @{
        version = 1
        repo = @{ name = "0lucasstavares/rato"; root = $repoRoot.Path }
        loop = @{
            status = "idle"
            interval_seconds = $IntervalSeconds
            pid = $PID
            started_ms = Get-NowMs
            last_tick_ms = $null
            current_role = $null
            current_run_id = $null
        }
        harness = @{
            command = if ($env:RATO_AGENT_COMMAND) { $env:RATO_AGENT_COMMAND } else { "pwsh ./scripts/agent/run-provider-agent.ps1" }
            preferred = if ($env:RATO_AGENT_HARNESS) { $env:RATO_AGENT_HARNESS } elseif ($env:RATO_AGENT_PROVIDER) { $env:RATO_AGENT_PROVIDER } else { "auto" }
            model = if ($env:RATO_AGENT_MODEL) { $env:RATO_AGENT_MODEL } else { "gpt-5.4-mini" }
        }
        queue = @{ ready = 0; working = 0; review = 0; merge = 0; blocked = 0; open_prs = 0 }
        pull_requests = @()
        workflows = @(
            $workflows.'scrum-master'
            $workflows.manager
            $workflows.worker
            $workflows.reviewer
            $workflows.merger
        )
        recent_runs = @()
        last_error = $null
        updated_ms = Get-NowMs
    }
}

function Write-State([hashtable]$State) {
    $State.updated_ms = Get-NowMs
    $json = $State | ConvertTo-Json -Depth 8
    Write-Utf8NoBom $statePath $json
}

function Get-WorkflowEntry([hashtable]$State, [string]$Role) {
    return @($State.workflows) | Where-Object { $_.workflow -eq "local-$Role" } | Select-Object -First 1
}

function Convert-LabelsToNames($Labels) {
    return @($Labels | ForEach-Object {
        if ($_ -is [string]) { $_ } elseif ($_.name) { $_.name }
    })
}

function Get-SafeRefSegment([string]$Value) {
    if (-not $Value) { return 'task' }
    $safe = $Value.ToLowerInvariant() -replace '[^a-z0-9._-]+', '-'
    $safe = $safe.Trim('-._')
    if (-not $safe) { return 'task' }
    return $safe
}

function Get-GitHubSnapshot {
    $issueJson = gh issue list --repo 0lucasstavares/rato --state open --limit 100 --json number,title,labels
    if ($LASTEXITCODE -ne 0) { throw "Failed to list issues with gh." }
    $prJson = gh pr list --repo 0lucasstavares/rato --state open --limit 50 --json number,title,isDraft,mergeStateStatus,updatedAt,url,labels,statusCheckRollup
    if ($LASTEXITCODE -ne 0) { throw "Failed to list pull requests with gh." }

    $issues = @(Convert-JsonArray $issueJson)
    $prs = @(Convert-JsonArray $prJson)

    $queue = @{ ready = 0; working = 0; review = 0; merge = 0; blocked = 0; open_prs = $prs.Count }
    $readyIssues = @()
    foreach ($issue in $issues) {
        $labels = Convert-LabelsToNames $issue.labels
        if ($labels -contains 'ai:ready') {
            $queue.ready += 1
            $readyIssues += @(@{ number = $issue.number; title = $issue.title; labels = $labels })
        }
        if ($labels -contains 'ai:working') { $queue.working += 1 }
        if ($labels -contains 'ai:review') { $queue.review += 1 }
        if ($labels -contains 'ai:merge') { $queue.merge += 1 }
        if ($labels -contains 'ai:blocked') { $queue.blocked += 1 }
    }

    $prDtos = @($prs | ForEach-Object {
        @{
            number = $_.number
            title = $_.title
            draft = [bool]$_.isDraft
            merge_state = $_.mergeStateStatus
            updated_at = $_.updatedAt
            url = $_.url
            labels = @(Convert-LabelsToNames $_.labels)
            checks = @($_.statusCheckRollup)
        }
    })

    return @{ queue = $queue; pull_requests = $prDtos; ready_issues = $readyIssues }
}

function Sync-QueueToState([hashtable]$State, [hashtable]$Snapshot) {
    $State.queue = $Snapshot.queue
    $State.pull_requests = $Snapshot.pull_requests
}

function Set-WorkflowStatus([hashtable]$State, [string]$Role, [string]$Status, [string]$Result, [Nullable[long]]$AtMs) {
    $entry = Get-WorkflowEntry $State $Role
    if (-not $entry) { return }
    $entry.status = $Status
    if ($Result) { $entry.last_result = $Result }
    if ($AtMs) { $entry.last_run_ms = $AtMs }
}

function Add-RecentRun([hashtable]$State, [hashtable]$RunRecord) {
    $recent = @($RunRecord) + @($State.recent_runs)
    if ($recent.Count -gt 20) { $recent = $recent[0..19] }
    $State.recent_runs = $recent
}

function Invoke-Checked([string]$Exe, [string[]]$Arguments) {
    & $Exe @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "Command failed with exit code ${LASTEXITCODE}: $Exe $($Arguments -join ' ')"
    }
}

function Get-BaseBranch {
    $current = @(git branch --show-current)
    if ($LASTEXITCODE -eq 0 -and $current.Count -gt 0) {
        $name = $current[0].Trim()
        if ($name -and $name -notlike 'ai/*') {
            return $name
        }
    }
    $hasMain = @(git branch --list main)
    if ($hasMain.Count -gt 0) { return 'main' }
    $hasMaster = @(git branch --list master)
    if ($hasMaster.Count -gt 0) { return 'master' }
    throw 'Unable to determine a base branch for worker setup.'
}

function Ensure-PathWithinWorktreesRoot([string]$PathToCheck) {
    $resolvedRoot = [System.IO.Path]::GetFullPath($worktreesRoot)
    $resolvedPath = [System.IO.Path]::GetFullPath($PathToCheck)
    if (-not $resolvedPath.StartsWith($resolvedRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to operate outside autonomy worktrees root: $resolvedPath"
    }
}

function Ensure-WorkerWorktree([hashtable]$Snapshot) {
    $issue = @($Snapshot.ready_issues) | Select-Object -First 1
    if (-not $issue) {
        throw 'Worker requested but no ai:ready issue was found.'
    }

    $slug = Get-SafeRefSegment $issue.title
    $branch = "ai/issue-$($issue.number)-$slug"
    $worktreePath = Join-Path $worktreesRoot "issue-$($issue.number)-$slug"
    Ensure-PathWithinWorktreesRoot $worktreePath

    if (Test-Path -LiteralPath (Join-Path $worktreePath '.git')) {
        return @{ issue = $issue; branch = $branch; path = $worktreePath }
    }

    if (Test-Path -LiteralPath $worktreePath) {
        Remove-Item -LiteralPath $worktreePath -Recurse -Force
    }

    $baseBranch = Get-BaseBranch
    $existingBranch = @(git branch --list $branch)
    if ($existingBranch.Count -gt 0) {
        Invoke-Checked 'git' @('worktree', 'add', $worktreePath, $branch)
    } else {
        Invoke-Checked 'git' @('worktree', 'add', '-b', $branch, $worktreePath, $baseBranch)
    }

    return @{ issue = $issue; branch = $branch; path = $worktreePath }
}

function Invoke-AgentRole([hashtable]$State, [string]$Role, [string]$RoleRepoRoot = $repoRoot) {
    $startedMs = Get-NowMs
    $runId = "{0}-{1}" -f $Role, ([Guid]::NewGuid().ToString('N').Substring(0, 10))
    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    $logPath = Join-Path $logsDir "$stamp-$Role.log"

    $State.loop.current_role = $Role
    $State.loop.current_run_id = $runId
    Set-WorkflowStatus -State $State -Role $Role -Status 'running' -Result 'running locally' -AtMs $startedMs
    Write-State $State

    $previousCommand = $env:RATO_AGENT_COMMAND
    $previousIdentity = $env:RATO_AGENT_ID
    $previousAutonomy = $env:RATO_AUTONOMY
    if (-not $env:RATO_AGENT_COMMAND) { $env:RATO_AGENT_COMMAND = 'pwsh ./scripts/agent/run-provider-agent.ps1' }
    $env:RATO_AGENT_ID = "local-$Role"
    $env:RATO_AUTONOMY = 'local'

    try {
        Push-Location $RoleRepoRoot
        $roleScript = Join-Path $RoleRepoRoot 'scripts\agent\run-agent-role.ps1'
        $output = & powershell -ExecutionPolicy Bypass -File $roleScript -Role $Role 2>&1
        $exitCode = $LASTEXITCODE
    } finally {
        Pop-Location
        if ($null -eq $previousCommand) { Remove-Item Env:RATO_AGENT_COMMAND -ErrorAction SilentlyContinue } else { $env:RATO_AGENT_COMMAND = $previousCommand }
        if ($null -eq $previousIdentity) { Remove-Item Env:RATO_AGENT_ID -ErrorAction SilentlyContinue } else { $env:RATO_AGENT_ID = $previousIdentity }
        if ($null -eq $previousAutonomy) { Remove-Item Env:RATO_AUTONOMY -ErrorAction SilentlyContinue } else { $env:RATO_AUTONOMY = $previousAutonomy }
    }

    $endedMs = Get-NowMs
    $durationMs = $endedMs - $startedMs
    $lines = @($output | ForEach-Object { "$_" })
    Write-Utf8NoBom $logPath ($lines -join [Environment]::NewLine)

    $summary = if ($exitCode -eq 0) { 'completed' } else { "failed with exit code $exitCode" }
    $runRecord = @{
        id = $runId
        role = $Role
        workflow = "local-$Role"
        status = if ($exitCode -eq 0) { 'passed' } else { 'failed' }
        started_ms = $startedMs
        ended_ms = $endedMs
        runtime_ms = $durationMs
        exit_code = $exitCode
        result_summary = $summary
        log_path = $logPath
    }

    Add-RecentRun -State $State -RunRecord $runRecord
    Set-WorkflowStatus -State $State -Role $Role -Status $runRecord.status -Result $summary -AtMs $endedMs
    $State.loop.current_role = $null
    $State.loop.current_run_id = $null
    if ($exitCode -eq 0) {
        $State.last_error = $null
    } else {
        $State.last_error = @{ role = $Role; message = $summary; at_ms = $endedMs; log_path = $logPath }
    }
    Write-State $State

    if ($exitCode -ne 0) { throw "Role '$Role' failed. See $logPath" }
}

function Invoke-Tick([hashtable]$State) {
    $State.loop.status = 'running'
    $State.loop.last_tick_ms = Get-NowMs
    Write-State $State

    $executed = @{}
    while ($true) {
        $snapshot = Get-GitHubSnapshot
        Sync-QueueToState -State $State -Snapshot $snapshot
        Write-State $State

        $role = $null
        if (-not $executed.ContainsKey('scrum-master')) {
            $role = 'scrum-master'
        } elseif ($snapshot.queue.open_prs -gt 0) {
            if (-not $executed.ContainsKey('reviewer')) {
                $role = 'reviewer'
            } elseif (-not $executed.ContainsKey('merger')) {
                $role = 'merger'
            }
        } else {
            if (-not $executed.ContainsKey('manager')) {
                $role = 'manager'
            } elseif ($snapshot.queue.ready -gt 0 -and -not $executed.ContainsKey('worker')) {
                $role = 'worker'
            }
        }

        if (-not $role) { break }

        if ($role -eq 'worker') {
            $workerContext = Ensure-WorkerWorktree -Snapshot $snapshot
            Invoke-AgentRole -State $State -Role $role -RoleRepoRoot $workerContext.path
        } else {
            Invoke-AgentRole -State $State -Role $role
        }
        $executed[$role] = $true
    }

    $finalSnapshot = Get-GitHubSnapshot
    Sync-QueueToState -State $State -Snapshot $finalSnapshot
    $State.loop.status = 'idle'
    Write-State $State
}

Require-Command 'git'
Require-Command 'gh'
Ensure-StateLayout

$state = Read-State
if (-not $state) { $state = New-State }
$state.loop.interval_seconds = $IntervalSeconds
$state.loop.pid = $PID
$state.harness.command = if ($env:RATO_AGENT_COMMAND) { $env:RATO_AGENT_COMMAND } else { 'pwsh ./scripts/agent/run-provider-agent.ps1' }
$state.harness.preferred = if ($env:RATO_AGENT_HARNESS) { $env:RATO_AGENT_HARNESS } elseif ($env:RATO_AGENT_PROVIDER) { $env:RATO_AGENT_PROVIDER } else { 'auto' }
$state.harness.model = if ($env:RATO_AGENT_MODEL) { $env:RATO_AGENT_MODEL } else { 'gpt-5.4-mini' }
Write-State $state

if ($Once) {
    Invoke-Tick -State $state
    exit 0
}

while ($true) {
    try {
        Invoke-Tick -State $state
    } catch {
        $state.loop.status = 'error'
        $state.last_error = @{ role = $state.loop.current_role; message = $_.Exception.Message; at_ms = Get-NowMs; log_path = $null }
        $state.loop.current_role = $null
        $state.loop.current_run_id = $null
        Write-State $state
    }
    Start-Sleep -Seconds $IntervalSeconds
}