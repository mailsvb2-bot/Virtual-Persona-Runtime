param(
    [int]$Port = 8787,
    [switch]$NoBrowser
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$baseUrl = "http://127.0.0.1:$Port"
$origin = $baseUrl
$tempRoot = Join-Path $env:TEMP ("vpr-owner-lab-restart-" + [guid]::NewGuid().ToString('N'))
$personaPath = Join-Path $tempRoot 'reviewed-persona.json'
$preservedPersona = $null

function Get-Bootstrap {
    Invoke-RestMethod -Method Get -Uri "$baseUrl/api/bootstrap" -TimeoutSec 2
}

function Get-LabStatus {
    Invoke-RestMethod -Method Get -Uri "$baseUrl/api/status" -TimeoutSec 2
}

function Invoke-LabPost {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)]$Body,
        [Parameter(Mandatory = $true)][string]$CsrfToken
    )

    $headers = @{
        'Origin' = $origin
        'X-VPR-CSRF' = $CsrfToken
    }
    $json = $Body | ConvertTo-Json -Compress -Depth 20
    Invoke-RestMethod -Method Post -Uri "$baseUrl$Path" -Headers $headers -ContentType 'application/json' -Body $json -TimeoutSec 15
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

function Export-ReviewedPersonaIfPresent {
    if (-not (Test-LabReachable)) {
        return $null
    }

    $status = Get-LabStatus
    if ($status.session_state -notin @('none', 'closed')) {
        throw "Owner Lab has an active/non-terminal session ($($status.session_state)); refusing to force-restart and lose session evidence"
    }
    if ($status.owner_context_state -ne 'reviewed') {
        return $null
    }

    $bootstrap = Get-Bootstrap
    $profile = Invoke-LabPost -Path '/api/persona/reviewed' -Body @{} -CsrfToken $bootstrap.csrf_token
    if ($profile.persona_version -ne 2) {
        throw "Reviewed Persona version $($profile.persona_version) cannot be losslessly replayed by the RT0 restart bridge; leaving the current process untouched"
    }
    if (@($profile.claims).Count -ne 3) {
        throw "Reviewed Persona does not contain the canonical 3 RT0 claims; leaving the current process untouched"
    }

    New-Item -ItemType Directory -Force -Path $tempRoot | Out-Null
    $profile | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath $personaPath -Encoding UTF8
    Write-Host "Preserved reviewed Persona in a temporary local file."
    return $profile
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

function Restore-ReviewedPersona {
    param([Parameter(Mandatory = $true)]$Profile)

    $claimsById = @{}
    foreach ($claim in @($Profile.claims)) {
        $claimsById[$claim.claim_id] = $claim
    }

    $orderedIds = @(
        'identity-self-description',
        'preference-communication-style',
        'opinion-core-principle'
    )
    foreach ($claimId in $orderedIds) {
        if (-not $claimsById.ContainsKey($claimId)) {
            throw "Preserved Persona is missing canonical claim $claimId"
        }
    }

    $bootstrap = Get-Bootstrap
    $csrf = $bootstrap.csrf_token
    $null = Invoke-LabPost -Path '/api/persona/create' -Body @{ persona_id = $Profile.persona_id } -CsrfToken $csrf
    foreach ($claimId in $orderedIds) {
        $claim = $claimsById[$claimId]
        $null = Invoke-LabPost -Path '/api/persona/capture/answer' -Body @{ answer = $claim.statement } -CsrfToken $csrf
    }
    $null = Invoke-LabPost -Path '/api/persona/capture/finish' -Body @{} -CsrfToken $csrf
    foreach ($claimId in $orderedIds) {
        $claim = $claimsById[$claimId]
        $null = Invoke-LabPost -Path '/api/persona/claims/correct' -Body @{
            claim_id = $claimId
            statement = $claim.statement
            kind = $claim.kind
        } -CsrfToken $csrf
    }
    $null = Invoke-LabPost -Path '/api/persona/review/complete' -Body @{} -CsrfToken $csrf

    $restored = Invoke-LabPost -Path '/api/persona/reviewed' -Body @{} -CsrfToken $csrf
    if ($restored.persona_id -ne $Profile.persona_id -or $restored.persona_version -ne 2) {
        throw 'Restored Persona identity/version does not match the preserved profile'
    }
    foreach ($claimId in $orderedIds) {
        $before = $claimsById[$claimId]
        $after = @($restored.claims | Where-Object { $_.claim_id -eq $claimId })[0]
        if ($null -eq $after -or $after.statement -ne $before.statement -or $after.kind -ne $before.kind) {
            throw "Restored Persona claim $claimId does not match the preserved profile"
        }
    }
    Write-Host "Restored reviewed Persona $($restored.persona_id), version $($restored.persona_version)."
}

try {
    $preservedPersona = Export-ReviewedPersonaIfPresent
    Stop-PortListener

    Push-Location $repoRoot
    try {
        git switch main
        if ($LASTEXITCODE -ne 0) { throw 'git switch main failed' }
        git pull --ff-only
        if ($LASTEXITCODE -ne 0) { throw 'git pull --ff-only failed' }
        cargo build -p vpr-owner-lab --bin vpr-owner-lab
        if ($LASTEXITCODE -ne 0) { throw 'Owner Lab build failed' }
    } finally {
        Pop-Location
    }

    $exe = Join-Path $repoRoot 'target\debug\vpr-owner-lab.exe'
    if (-not (Test-Path -LiteralPath $exe)) {
        throw "Owner Lab executable was not produced: $exe"
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

    if ($null -ne $preservedPersona) {
        Restore-ReviewedPersona -Profile $preservedPersona
    }

    $status = Get-LabStatus
    Write-Host "Owner Lab ready: pid=$listenerPid port=$Port egress=$($status.egress_enabled), conversation=$($status.conversation_readiness), persona=$($status.owner_context_state)."
    if (-not $NoBrowser) {
        Start-Process $baseUrl
    }
} finally {
    if (Test-Path -LiteralPath $tempRoot) {
        Remove-Item -LiteralPath $tempRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}
