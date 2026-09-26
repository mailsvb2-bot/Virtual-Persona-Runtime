param(
    [int]$Port = 8787,
    [switch]$NoBrowser,
    [switch]$MigrationSelfTest
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$baseUrl = "http://127.0.0.1:$Port"
$origin = $baseUrl
$tempRoot = Join-Path $env:TEMP ("vpr-owner-lab-restart-" + [guid]::NewGuid().ToString('N'))
$personaPath = Join-Path $tempRoot 'reviewed-persona.json'
$profileCacheRoot = Join-Path $env:LOCALAPPDATA 'Virtual-Persona-Runtime\owner-lab'
$profileCachePath = Join-Path $profileCacheRoot 'reviewed-persona.dpapi'
$legacyProfilePaths = @(
    'C:\VPR-RT0\input\reviewed-profile.json',
    'C:\VPR-RT0\reviewed-profile.json'
)
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

function Save-ReviewedPersonaCache {
    param([Parameter(Mandatory = $true)]$Profile)

    New-Item -ItemType Directory -Force -Path $profileCacheRoot | Out-Null
    $json = $Profile | ConvertTo-Json -Compress -Depth 20
    $plain = [System.Text.Encoding]::UTF8.GetBytes($json)
    $protected = [System.Security.Cryptography.ProtectedData]::Protect(
        $plain,
        $null,
        [System.Security.Cryptography.DataProtectionScope]::CurrentUser
    )
    [System.IO.File]::WriteAllBytes($profileCachePath, $protected)
    Write-Host "Cached reviewed Persona with Windows DPAPI for restart recovery."
}

function Read-ReviewedPersonaCache {
    if (-not (Test-Path -LiteralPath $profileCachePath)) {
        return $null
    }

    try {
        $protected = [System.IO.File]::ReadAllBytes($profileCachePath)
        $plain = [System.Security.Cryptography.ProtectedData]::Unprotect(
            $protected,
            $null,
            [System.Security.Cryptography.DataProtectionScope]::CurrentUser
        )
        $json = [System.Text.Encoding]::UTF8.GetString($plain)
        return $json | ConvertFrom-Json
    } catch {
        throw "Reviewed Persona cache exists but could not be decrypted for the current Windows user"
    }
}

function Convert-ToImportableReviewedPersona {
    param([Parameter(Mandatory = $true)]$Profile)

    if ([string]::IsNullOrWhiteSpace([string]$Profile.persona_id)) {
        throw 'Reviewed Persona is missing persona_id'
    }

    $claims = @()
    foreach ($claim in @($Profile.claims)) {
        if ([string]::IsNullOrWhiteSpace([string]$claim.claim_id) -or
            [string]::IsNullOrWhiteSpace([string]$claim.statement) -or
            [string]::IsNullOrWhiteSpace([string]$claim.kind)) {
            throw 'Reviewed Persona contains an incomplete claim'
        }
        if ($null -ne $claim.owner_approved -and -not [bool]$claim.owner_approved) {
            continue
        }
        $claims += [pscustomobject]@{
            claim_id = [string]$claim.claim_id
            statement = [string]$claim.statement
            kind = [string]$claim.kind
        }
    }
    if ($claims.Count -eq 0) {
        throw 'Reviewed Persona contains no owner-approved claims'
    }

    return [pscustomobject]@{
        persona_id = [string]$Profile.persona_id
        claims = $claims
    }
}

function Invoke-MigrationSelfTest {
    $legacy = [pscustomobject]@{
        persona_id = 'legacy-owner-self-test'
        owner_review_confirmed = $true
        claims = @(
            [pscustomobject]@{
                claim_id = 'identity-self-description'
                statement = 'owner identity self test'
                kind = 'factual'
                owner_approved = $true
            },
            [pscustomobject]@{
                claim_id = 'preference-communication-style'
                statement = 'concise communication'
                kind = 'preference'
                owner_approved = $true
            },
            [pscustomobject]@{
                claim_id = 'ignored-unapproved'
                statement = 'must not be imported'
                kind = 'opinion'
                owner_approved = $false
            }
        )
    }

    $converted = Convert-ToImportableReviewedPersona -Profile $legacy
    if ($converted.persona_id -ne 'legacy-owner-self-test') {
        throw 'Migration self-test changed persona_id'
    }
    if (@($converted.claims).Count -ne 2) {
        throw "Migration self-test expected 2 approved claims, got $(@($converted.claims).Count)"
    }
    if (@($converted.claims | Where-Object { $_.claim_id -eq 'ignored-unapproved' }).Count -ne 0) {
        throw 'Migration self-test imported an unapproved legacy claim'
    }
    if (@($converted.claims | Where-Object { $_.claim_id -eq 'identity-self-description' }).Count -ne 1) {
        throw 'Migration self-test lost identity claim'
    }
    Write-Host 'Reviewed Persona migration self-test passed.'
}

function Read-LegacyReviewedPersona {
    foreach ($legacyProfilePath in $legacyProfilePaths) {
        if (-not (Test-Path -LiteralPath $legacyProfilePath)) {
            continue
        }

        try {
            $profile = Get-Content -LiteralPath $legacyProfilePath -Raw -Encoding UTF8 | ConvertFrom-Json
            if ($null -ne $profile.owner_review_confirmed -and -not [bool]$profile.owner_review_confirmed) {
                throw 'legacy owner review was not confirmed'
            }
            $importable = Convert-ToImportableReviewedPersona -Profile $profile
            Write-Host "Recovered reviewed Persona from legacy $legacyProfilePath."
            Save-ReviewedPersonaCache -Profile $importable
            return $importable
        } catch {
            throw "Legacy reviewed Persona exists at $legacyProfilePath but could not be migrated: $($_.Exception.Message)"
        }
    }
    return $null
}

function Get-CachedReviewedPersona {
    $cached = Read-ReviewedPersonaCache
    if ($null -ne $cached) {
        Write-Host "Recovered reviewed Persona from the persistent Windows user cache."
        return $cached
    }
    return Read-LegacyReviewedPersona
}

function Export-ReviewedPersonaIfPresent {
    if (-not (Test-LabReachable)) {
        return Get-CachedReviewedPersona
    }

    $status = Get-LabStatus
    if ($status.session_state -notin @('none', 'closed')) {
        throw "Owner Lab has an active/non-terminal session ($($status.session_state)); refusing to force-restart and lose session evidence"
    }
    if ($status.owner_context_state -ne 'reviewed') {
        return Get-CachedReviewedPersona
    }

    $bootstrap = Get-Bootstrap
    $profile = Invoke-LabPost -Path '/api/persona/reviewed' -Body @{} -CsrfToken $bootstrap.csrf_token
    $importable = Convert-ToImportableReviewedPersona -Profile $profile
    New-Item -ItemType Directory -Force -Path $tempRoot | Out-Null
    $importable | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath $personaPath -Encoding UTF8
    Save-ReviewedPersonaCache -Profile $importable
    Write-Host "Preserved reviewed Persona for this restart and future restarts."
    return $importable
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

function Restore-ReviewedPersona {
    param([Parameter(Mandatory = $true)]$Profile)

    $importable = Convert-ToImportableReviewedPersona -Profile $Profile
    $bootstrap = Get-Bootstrap
    $csrf = $bootstrap.csrf_token
    $null = Invoke-LabPost -Path '/api/persona/reviewed/import' -Body $importable -CsrfToken $csrf

    $restored = Invoke-LabPost -Path '/api/persona/reviewed' -Body @{} -CsrfToken $csrf
    if ($restored.persona_id -ne $importable.persona_id) {
        throw 'Restored Persona identity does not match the preserved profile'
    }
    if (@($restored.claims).Count -ne @($importable.claims).Count) {
        throw 'Restored Persona claim count does not match the preserved profile'
    }
    foreach ($claim in @($importable.claims)) {
        $after = @($restored.claims | Where-Object { $_.claim_id -eq $claim.claim_id })[0]
        if ($null -eq $after -or $after.statement -ne $claim.statement -or $after.kind -ne $claim.kind) {
            throw "Restored Persona claim $($claim.claim_id) does not match the preserved profile"
        }
    }
    Save-ReviewedPersonaCache -Profile $importable
    Write-Host "Restored reviewed Persona $($restored.persona_id), version $($restored.persona_version), claims=$(@($restored.claims).Count)."
}

if ($MigrationSelfTest) {
    Invoke-MigrationSelfTest
    exit 0
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
    & $credentialsExe probe-did
    if ($LASTEXITCODE -ne 0) {
        throw 'D-ID credential preflight failed; Owner Lab was not started'
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
    if ($null -ne $preservedPersona -and
        ($status.owner_context_state -ne 'reviewed' -or $status.reviewed_owner_claims -lt 1)) {
        throw 'Reviewed Persona was found before restart but was not restored; refusing to open the qualification UI'
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
