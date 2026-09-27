param(
    [int]$Port = 8787,
    [switch]$NoBrowser
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$baseUrl = "http://127.0.0.1:$Port"

function Get-Bootstrap {
    Invoke-RestMethod -Method Get -Uri "$baseUrl/api/bootstrap" -TimeoutSec 2
}

function Get-LabStatus {
    Invoke-RestMethod -Method Get -Uri "$baseUrl/api/status" -TimeoutSec 2
}

function Test-LabReachable {
    try {
        $null = Get-Bootstrap
        return $true
    } catch {
        return $false
    }
}

function Wait-LabUp {
    param([int]$TimeoutSeconds = 120)
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        if (Test-LabReachable) {
            return
        }
        Start-Sleep -Milliseconds 500
    }
    throw "Owner Lab did not start on $baseUrl within $TimeoutSeconds seconds"
}

function Wait-LabDown {
    param([int]$TimeoutSeconds = 15)
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        if (-not (Test-LabReachable)) {
            return
        }
        Start-Sleep -Milliseconds 250
    }
    throw "Old Owner Lab is still listening on $baseUrl"
}

function Assert-SafeToRestart {
    if (-not (Test-LabReachable)) {
        return
    }

    $status = Get-LabStatus
    if ($status.session_state -notin @('none', 'closed')) {
        throw "Owner Lab has an active/non-terminal session ($($status.session_state)); refusing to force-restart and lose session evidence"
    }
}

function Stop-PortListener {
    $listeners = @(Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue)
    $pids = @($listeners | Select-Object -ExpandProperty OwningProcess -Unique)
    foreach ($processId in $pids) {
        if ($processId -and $processId -ne $PID) {
            Write-Host "Stopping stale Owner Lab listener PID $processId on port $Port."
            Stop-Process -Id $processId -Force -ErrorAction Stop
        }
    }
    if ($pids.Count -gt 0) {
        Wait-LabDown
    }
}

function Assert-ExpectedListener {
    param([Parameter(Mandatory = $true)][string]$ExpectedExe)

    $listeners = @(Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue)
    if ($listeners.Count -ne 1) {
        throw "Expected exactly one listener on port $Port, found $($listeners.Count)"
    }

    $listenerPid = $listeners[0].OwningProcess
    $process = Get-Process -Id $listenerPid -ErrorAction Stop
    $actualExe = $process.Path
    if ([string]::IsNullOrWhiteSpace($actualExe)) {
        throw "Could not resolve executable for listener PID $listenerPid"
    }

    $expectedPath = [System.IO.Path]::GetFullPath($ExpectedExe)
    $actualPath = [System.IO.Path]::GetFullPath($actualExe)
    if (-not [string]::Equals($expectedPath, $actualPath, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Port $Port is owned by unexpected process: $actualPath"
    }
    return $listenerPid
}

function Clear-ProviderEnvironmentOverrides {
    $names = @(
        'VPR_DID_ENDPOINT',
        'VPR_DID_API_KEY',
        'VPR_DID_AGENT_ID',
        'VPR_DID_FLUENT',
        'VPR_OWNER_LAB_AVATAR_PROVIDER',
        'VPR_LOCAL_AVATAR_ENDPOINT',
        'VPR_LOCAL_AVATAR_API_TOKEN',
        'VPR_OWNER_LAB_STT_PROVIDER',
        'VPR_OWNER_LAB_STT_ENDPOINT',
        'VPR_OWNER_LAB_STT_API_KEY',
        'VPR_OWNER_LAB_STT_MODEL',
        'VPR_OWNER_LAB_LLM_PROVIDER',
        'VPR_OWNER_LAB_LLM_ENDPOINT',
        'VPR_OWNER_LAB_LLM_API_KEY',
        'VPR_OWNER_LAB_LLM_MODEL'
    )
    foreach ($name in $names) {
        Remove-Item -Path "Env:$name" -ErrorAction SilentlyContinue
    }
    Write-Host "Cleared inherited provider env overrides; canonical Windows credential profile is authoritative."
}

Assert-SafeToRestart
Stop-PortListener

try {

    Push-Location $repoRoot
    try {
        git switch main
        if ($LASTEXITCODE -ne 0) { throw 'git switch main failed' }
        git pull --ff-only
        if ($LASTEXITCODE -ne 0) { throw 'git pull --ff-only failed' }
        cargo build -p vpr-owner-lab --bins
        if ($LASTEXITCODE -ne 0) { throw 'Owner Lab binaries build failed' }
    } finally {
        Pop-Location
    }

    $exe = Join-Path $repoRoot 'target\debug\vpr-owner-lab.exe'
    $credentialsExe = Join-Path $repoRoot 'target\debug\vpr-provider-credentials.exe'
    if (-not (Test-Path -LiteralPath $exe)) {
        throw "Owner Lab executable was not produced: $exe"
    }
    if (-not (Test-Path -LiteralPath $credentialsExe)) {
        throw "Provider credential diagnostic was not produced: $credentialsExe"
    }

    Clear-ProviderEnvironmentOverrides
    & $credentialsExe probe-avatar
    if ($LASTEXITCODE -ne 0) {
        throw 'Avatar provider preflight failed; Owner Lab was not started'
    }

    $cmd = "set `"VPR_OWNER_LAB_ALLOW_EGRESS=true`" && set `"VPR_OWNER_LAB_PORT=$Port`" && `"$exe`" --allow-egress"
    Start-Process -FilePath 'cmd.exe' -ArgumentList '/k', $cmd -WorkingDirectory $repoRoot | Out-Null
    Wait-LabUp

    $listenerPid = Assert-ExpectedListener -ExpectedExe $exe
    $bootstrap = Get-Bootstrap
    $status = Get-LabStatus
    if (-not $bootstrap.egress_enabled -or -not $status.egress_enabled) {
        throw 'New Owner Lab started, but backend egress is still disabled'
    }
    if ($status.conversation_readiness -ne 'text_and_voice') {
        throw "Provider profile is incomplete: conversation_readiness=$($status.conversation_readiness)"
    }

    $status = Get-LabStatus
    Write-Host "Owner Lab ready: pid=$listenerPid port=$Port egress=$($status.egress_enabled), conversation=$($status.conversation_readiness), persona=$($status.owner_context_state)."
    if (-not $NoBrowser) {
        Start-Process $baseUrl
    }
} catch {
    throw
}
