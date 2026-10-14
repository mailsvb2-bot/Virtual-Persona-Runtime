param(
    [int]$Port = 8787,
    [switch]$NoBrowser,
    [switch]$Rt0Evidence,
    [switch]$Piper,
    [switch]$SystemVoice,
    [string]$VoiceProfile
)

$ErrorActionPreference = 'Stop'
if ($Piper -and $SystemVoice) { throw 'Select exactly one local speech engine' }
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if ($VoiceProfile -and ($Piper -or $SystemVoice)) {
    throw 'choose one voice source: -VoiceProfile cannot be combined with -Piper/-SystemVoice'
}
if ($VoiceProfile) {
    if (-not (Test-Path -LiteralPath $VoiceProfile -PathType Leaf)) {
        throw 'VOICE_PROFILE_UNAVAILABLE: path must point to a readable JSON file'
    }
    $resolvedVoiceProfile = (Resolve-Path -LiteralPath $VoiceProfile).Path
    $repositoryPrefix = $repoRoot.TrimEnd('\') + '\'
    if ($resolvedVoiceProfile.StartsWith($repositoryPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'VOICE_PROFILE_INSIDE_REPOSITORY_FORBIDDEN'
    }
    $voiceInfo = Get-Item -LiteralPath $resolvedVoiceProfile
    if ($voiceInfo.Length -gt 16384) { throw 'VOICE_PROFILE_TOO_LARGE' }
    try { $voiceConfig = Get-Content -LiteralPath $resolvedVoiceProfile -Raw -Encoding UTF8 | ConvertFrom-Json }
    catch { throw 'INVALID_VOICE_PROFILE_JSON' }
    if ($voiceConfig.schema_version -ne 'vpr-voice-profile-0.1' -or
        $voiceConfig.profile_id -notmatch '^[a-z0-9-]{3,64}
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
    param([Parameter(Mandatory = $true)][string]$ExpectedExe)

    $listeners = @(Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue)
    if ($listeners.Count -eq 0) {
        return
    }

    # Never kill an unrelated service just because it occupies Owner Lab's port.
    # This also rejects ambiguous/multiple listeners and unknown executable paths.
    $verifiedProcess = Assert-ExpectedListener -ExpectedExe $ExpectedExe -PassThruProcess
    try {
        if ($verifiedProcess.Id -eq $PID) {
            throw "Refusing to stop the current PowerShell process"
        }
        Write-Host "Stopping verified stale Owner Lab listener PID $($verifiedProcess.Id) on port $Port."
        # The acquired process Handle pins its Windows process object across
        # verification and Kill; a recycled numeric PID must never be resolved.
        $verifiedProcess.Kill()
    } finally {
        $verifiedProcess.Close()
    }
    Wait-LabDown
}

function Assert-ExpectedListener {
    param(
        [Parameter(Mandatory = $true)][string]$ExpectedExe,
        [int]$ListenPort = $Port,
        [switch]$PassThruProcess
    )

    $listeners = @(Get-NetTCPConnection -LocalPort $ListenPort -State Listen -ErrorAction SilentlyContinue)
    if ($listeners.Count -ne 1) {
        throw "Expected exactly one listener on port $ListenPort, found $($listeners.Count)"
    }

    $listenerPid = $listeners[0].OwningProcess
    $process = Get-Process -Id $listenerPid -ErrorAction Stop
    $handedOff = $false
    try {
        # Acquire a real process handle BEFORE reading its executable identity.
        # Retaining that handle prevents Windows from recycling the PID while
        # the checked process is being stopped.
        $null = $process.Handle
        $actualExe = $process.Path
        if ([string]::IsNullOrWhiteSpace($actualExe)) {
            throw "Could not resolve executable for listener PID $listenerPid"
        }

        $expectedPath = [System.IO.Path]::GetFullPath($ExpectedExe)
        $actualPath = [System.IO.Path]::GetFullPath($actualExe)
        if (-not [string]::Equals($expectedPath, $actualPath, [System.StringComparison]::OrdinalIgnoreCase)) {
            throw "Port $ListenPort is owned by unexpected process: $actualPath"
        }
        if ($PassThruProcess) {
            $handedOff = $true
            return $process
        }
        return $listenerPid
    } finally {
        if (-not $handedOff) {
            $process.Close()
        }
    }
}

function Get-VerifiedExistingOwnerLabExe {
    param([string]$LegacyExe, [string]$SlotAExe, [string]$SlotBExe)
    $listeners = @(Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue)
    if ($listeners.Count -eq 0) { return $null }
    if ($listeners.Count -ne 1) { throw "Ambiguous listener on $Port" }
    $process = Get-Process -Id $listeners[0].OwningProcess -ErrorAction Stop
    try {
        $null = $process.Handle
        $actual = $process.Path
        if ([string]::IsNullOrWhiteSpace($actual)) { throw 'Unverifiable Owner Lab process' }
        foreach ($candidate in @($LegacyExe, $SlotAExe, $SlotBExe)) {
            $expected = [System.IO.Path]::GetFullPath($candidate)
            if ([string]::Equals([System.IO.Path]::GetFullPath($actual), $expected, [System.StringComparison]::OrdinalIgnoreCase)) { return $expected }
        }
        throw "Foreign listener on Owner Lab port: $actual"
    } finally { $process.Close() }
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
        'VPR_OWNER_LAB_LLM_MODEL',
        'VPR_OWNER_LAB_RT0_EVIDENCE'
    )
    foreach ($name in $names) {
        Remove-Item -Path "Env:$name" -ErrorAction SilentlyContinue
    }
    if (-not $VoiceProfile) {
        Remove-Item Env:VPR_OWNER_LAB_VOICE_PROFILE_PATH -ErrorAction SilentlyContinue
    }
    Write-Host "Cleared inherited provider env overrides; canonical Windows credential profile is authoritative."
}

function New-PiperPrivateToken {
    $bytes = New-Object byte[] 32
    $rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
    try { $rng.GetBytes($bytes) } finally { $rng.Dispose() }
    return [Convert]::ToBase64String($bytes)
}

function Get-AvailableCanaryPort {
    param([int]$PrimaryPort = $Port)
    if ($PrimaryPort -lt 1 -or $PrimaryPort -gt 65503) {
        throw 'Owner Lab primary port cannot support a private canary port'
    }
    foreach ($candidate in ($PrimaryPort + 1)..($PrimaryPort + 32)) {
        if (@(Get-NetTCPConnection -LocalPort $candidate -State Listen -ErrorAction SilentlyContinue).Count -eq 0) {
            return [int]$candidate
        }
    }
    throw 'No free canary port; refusing to replace an existing Owner Lab'
}

function Assert-CanaryReadiness {
    param(
        [Parameter(Mandatory = $true)][object]$Bootstrap,
        [Parameter(Mandatory = $true)][object]$Status,
        [bool]$StrictEvidence
    )
    if (-not $Bootstrap.egress_enabled -or -not $Status.egress_enabled) {
        throw 'Staged Owner Lab did not acknowledge provider egress'
    }
    if ($StrictEvidence -and -not $Bootstrap.rt0_evidence_mode) {
        throw 'Staged Owner Lab did not acknowledge exact-candidate RT0 evidence mode'
    }
    if ($Status.conversation_readiness -ne 'text_and_voice') {
        throw "Staged Owner Lab is not text-and-voice ready: $($Status.conversation_readiness)"
    }
    if ($Status.session_state -notin @('none', 'closed')) {
        throw 'Staged Owner Lab must have no active session before handoff'
    }
}

function Wait-CanaryLabReady {
    param(
        [Parameter(Mandatory = $true)][object]$Process,
        [Parameter(Mandatory = $true)][int]$ListenPort,
        [Parameter(Mandatory = $true)][string]$ExpectedExe,
        [bool]$StrictEvidence,
        [int]$TimeoutSeconds = 45
    )
    $canaryUrl = "http://127.0.0.1:$ListenPort"
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $lastReadinessError = 'server never accepted the readiness requests'
    while ([DateTime]::UtcNow -lt $deadline) {
        if ($Process.HasExited) {
            throw 'Staged Owner Lab process exited before startup readiness'
        }
        $listeners = @(Get-NetTCPConnection -LocalPort $ListenPort -State Listen -ErrorAction SilentlyContinue)
        if ($listeners.Count -gt 0) {
            # Fail immediately on an unknown/ambiguous listener; NEVER probe it.
            $actualPid = Assert-ExpectedListener -ExpectedExe $ExpectedExe -ListenPort $ListenPort
            if ($actualPid -ne $Process.Id) {
                throw 'Private canary port is owned by a different process'
            }
            try {
                $bootstrap = Invoke-RestMethod -Method Get -Uri "$canaryUrl/api/bootstrap" -TimeoutSec 2
                $status = Invoke-RestMethod -Method Get -Uri "$canaryUrl/api/status" -TimeoutSec 2
                Assert-CanaryReadiness -Bootstrap $bootstrap -Status $status -StrictEvidence $StrictEvidence
                # A listening server without its embedded browser UI is not
                # a usable Owner Lab. Check both frontend entrypoints.
                $page = Invoke-WebRequest -UseBasicParsing -Method Get -Uri "$canaryUrl/" -TimeoutSec 2
                $bundle = Invoke-WebRequest -UseBasicParsing -Method Get -Uri "$canaryUrl/app.js" -TimeoutSec 2
                if ($page.StatusCode -ne 200 -or $bundle.StatusCode -ne 200 -or
                    [string]::IsNullOrWhiteSpace([string]$page.Content) -or
                    [string]::IsNullOrWhiteSpace([string]$bundle.Content)) {
                    throw 'Staged Owner Lab browser assets unavailable'
                }
                return
            } catch {
                $lastReadinessError = $_.Exception.Message
            }
        }
        Start-Sleep -Milliseconds 500
    }
    throw "Staged Owner Lab failed readiness before cutover: $lastReadinessError"
}

function Stop-CanaryLab {
    param(
        [Parameter(Mandatory = $true)][object]$Process,
        [Parameter(Mandatory = $true)][int]$ListenPort
    )
    # The Process object originates from Start-Process -PassThru; never
    # terminate a numeric PID looked up from a port or an untrusted file.
    try {
        if (-not $Process.HasExited) {
            $Process.Kill()
        }
        if (-not $Process.WaitForExit(15000)) {
            throw 'Staged Owner Lab failed to stop before the primary cutover'
        }
        # TCP teardown is asynchronous on Windows; allow a short bounded
        # release window rather than falsely failing an otherwise safe stop.
        $deadline = [DateTime]::UtcNow.AddSeconds(5)
        while ([DateTime]::UtcNow -lt $deadline) {
            if (@(Get-NetTCPConnection -LocalPort $ListenPort -State Listen -ErrorAction SilentlyContinue).Count -eq 0) {
                return
            }
            Start-Sleep -Milliseconds 125
        }
        throw 'Canary port still has a listener after process termination'
    } finally {
        $Process.Close()
    }
}

function Get-AvailablePrivateVoicePort {
    foreach ($candidate in 8897..8927) {
        if (@(Get-NetTCPConnection -LocalPort $candidate -State Listen -ErrorAction SilentlyContinue).Count -eq 0) { return [int]$candidate }
    }
    throw 'No free private voice port; existing Owner Lab was preserved'
}

function Test-VerifiedPiperListener {
    param(
        [Parameter(Mandatory = $true)][System.Diagnostics.Process]$Process,
        [Parameter(Mandatory = $true)][int]$OwnerPid,
        [Parameter(Mandatory = $true)][string]$VenvPython,
        [Parameter(Mandatory = $true)][string]$BasePython,
        [Parameter(Mandatory = $true)][string]$BridgeScript
    )
    if ($Process.HasExited) { return $false }
    # On Windows, a Python venv python.exe may be a redirector which creates
    # a child process running the base interpreter. Bind either case to the
    # original live process handle AND its expected executable / bridge path.
    $owner = Get-CimInstance Win32_Process -Filter "ProcessId = $OwnerPid" -ErrorAction SilentlyContinue
    if ($null -eq $owner) { return $false }
    $compare = [StringComparison]::OrdinalIgnoreCase
    if ($OwnerPid -eq $Process.Id) {
        return $owner.ExecutablePath -eq $VenvPython -and
            $owner.CommandLine -and $owner.CommandLine.IndexOf($BridgeScript, $compare) -ge 0
    }
    if ($owner.ParentProcessId -ne $Process.Id -or $owner.ExecutablePath -ne $BasePython -or
        -not $owner.CommandLine -or $owner.CommandLine.IndexOf($BridgeScript, $compare) -lt 0) {
        return $false
    }
    $parent = Get-CimInstance Win32_Process -Filter "ProcessId = $($Process.Id)" -ErrorAction SilentlyContinue
    return $null -ne $parent -and $parent.ExecutablePath -eq $VenvPython -and
        $parent.CommandLine -and $parent.CommandLine.IndexOf($BridgeScript, $compare) -ge 0
}

function Wait-PiperReady {
    param([Parameter(Mandatory = $true)][System.Diagnostics.Process]$Process)
    $deadline = [DateTime]::UtcNow.AddSeconds(75)
    $headers = @{ Authorization = "Bearer $env:VPR_METROTRANCE_TTS_TOKEN" }
    $selectedModel = if ($SystemVoice) { 'windows-sapi' } else { 'piper' }
    $body = '{"input":"Проверка голоса.","model":"' + $selectedModel + '","voice":"local","response_format":"wav"}'
    while ([DateTime]::UtcNow -lt $deadline) {
        if ($Process.HasExited) {
            throw 'Piper exited before its authenticated speech endpoint became ready'
        }
        # Check the actual Windows listener owner BEFORE sending the bearer
        # token. A foreign process racing the bind must never receive secrets.
        $listeners = @(Get-NetTCPConnection -LocalPort $privateVoicePort -State Listen -ErrorAction SilentlyContinue)
        if ($listeners.Count -gt 0) {
            if ($listeners.Count -ne 1 -or $listeners[0].LocalAddress -ne '127.0.0.1' -or
                -not (Test-VerifiedPiperListener -Process $Process -OwnerPid $listeners[0].OwningProcess -VenvPython $piperPython -BasePython $piperBasePython -BridgeScript $piperBridge)) {
                throw 'Piper port owner is not the verified spawned Python interpreter or its direct child'
            }
            $script:piperListenerPid = [int]$listeners[0].OwningProcess
        } else {
            Start-Sleep -Milliseconds 750
            continue
        }
        try {
            $result = Invoke-WebRequest -UseBasicParsing -Method Post -Uri ("http://127.0.0.1:$privateVoicePort/v1/audio/speech") -Headers $headers -ContentType 'application/json; charset=utf-8' -Body ([Text.Encoding]::UTF8.GetBytes($body)) -TimeoutSec 10
            if ($result.StatusCode -eq 200 -and $result.Headers['Content-Type'] -like 'audio/wav*') {
                return
            }
        } catch {
            # Connection can be refused while the model loads.
        }
        Start-Sleep -Milliseconds 750
    }
    throw 'Piper did not pass authenticated local speech warmup'
}

function Resolve-Rt0EvidenceBrowser {
    $programFilesX86 = [Environment]::GetFolderPath('ProgramFilesX86')
    $candidates = @()
    if ($env:ProgramFiles) {
        $candidates += Join-Path $env:ProgramFiles 'Mozilla Firefox\firefox.exe'
    }
    if ($programFilesX86) {
        $candidates += Join-Path $programFilesX86 'Mozilla Firefox\firefox.exe'
    }
    $candidates = @($candidates | Where-Object { Test-Path -LiteralPath $_ })

    $firefox = $candidates | Select-Object -First 1
    if (-not $firefox) {
        throw @"
RT0 exact-candidate A/V-sync evidence uses Firefox 142 or newer as the canonical evidence browser.
Strict RT0 A/V-sync evidence requires RTCInboundRtpStreamStats.estimatedPlayoutTimestamp.
HTML media-element currentTime is diagnostic-only and cannot qualify release evidence. Install Firefox 142 or newer, then rerun:
  powershell -NoProfile -ExecutionPolicy Bypass -File scripts\windows-owner-lab-restart.ps1 -Rt0Evidence
The Owner Lab backend was not opened in a non-evidence browser.
"@
    }

    $version = (Get-Item -LiteralPath $firefox).VersionInfo.ProductVersion
    $majorText = ($version -split '\.')[0]
    $major = 0
    if (-not [int]::TryParse($majorText, [ref]$major) -or $major -lt 142) {
        throw "RT0 exact-candidate evidence requires the canonical Firefox 142+ browser; found Firefox $version at $firefox"
    }

    Write-Host "RT0 evidence browser: Firefox $version ($firefox)"
    return $firefox
}

function Get-Rt0GitCandidate {
    param([Parameter(Mandatory = $true)][string]$RepositoryRoot)

    # ZIP/source-archive downloads lack .git metadata and cannot provide
    # verifiable exact-candidate provenance for release-evidence collection.
    if (-not (Test-Path -LiteralPath (Join-Path $RepositoryRoot '.git'))) {
        throw "RT0 evidence requires a Git clone, not a GitHub ZIP download. In CMD: git clone https://github.com/mailsvb2-bot/Virtual-Persona-Runtime.git ; then run this launcher from the cloned folder. No Owner Lab listener was stopped."
    }

    $candidateOutput = @(git -C $RepositoryRoot rev-parse --verify HEAD)
    if ($LASTEXITCODE -ne 0 -or $candidateOutput.Count -ne 1 -or $candidateOutput[0] -notmatch '^[0-9a-f]{40}$') {
        throw 'RT0 evidence requires a valid exact Git candidate; no Owner Lab listener was stopped'
    }
    return [string]$candidateOutput[0]
}

$evidenceBrowser = $null
if ($Rt0Evidence -and -not $NoBrowser) {
    $evidenceBrowser = Resolve-Rt0EvidenceBrowser
}

if ($Rt0Evidence) {
    Push-Location $repoRoot
    try {
        $candidate = Get-Rt0GitCandidate -RepositoryRoot $repoRoot
        $dirty = @(git status --porcelain --untracked-files=all)
        if ($LASTEXITCODE -ne 0 -or $dirty.Count -gt 0) {
            throw 'RT0 evidence requires a clean worktree; no listener was stopped'
        }
        Write-Host "Frozen RT0 candidate: $candidate (no Git switch or pull)"
    } finally {
        Pop-Location
    }
}


# Explicit opt-in download happens before stopping any existing session.
$piperPython = $null
$piperModel = $null
$privateVoicePort = 0
if ($Piper -or $SystemVoice) {
    if (-not $env:LOCALAPPDATA) {
        throw 'LocalAppData is required for the private Piper runtime'
    }
    $piperRoot = if ($SystemVoice) {
        Join-Path $env:LOCALAPPDATA 'Virtual-Persona-Runtime\Sapi'
    } else {
        Join-Path $env:LOCALAPPDATA 'Virtual-Persona-Runtime\Piper'
    }
    $preparer = Join-Path $repoRoot $(if ($SystemVoice) { 'scripts\windows_sapi_prepare.py' } else { 'scripts\windows_piper_prepare.py' })
    Write-Host $(if ($SystemVoice) { 'Preparing native Russian female voice and private Echo...' } else { 'Preparing small Russian Piper voice; only first use needs a download...' })
    & py -3.11 $preparer --data-dir $piperRoot
    if ($LASTEXITCODE -ne 0) {
        throw 'Piper setup failed before any Owner Lab session was stopped'
    }
    $piperPython = Join-Path $piperRoot 'venv\Scripts\python.exe'
    $piperModel = Join-Path $piperRoot 'voices\ru_RU-denis-medium.onnx'
    # Windows PowerShell 5.1 can corrupt non-ASCII paths emitted by native
    # Python stdout (e.g. C:\Users\Пользователь). Read Python's UTF-8 venv
    # configuration directly instead of guessing via terminal code pages.
    $venvConfig = Join-Path $piperRoot 'venv\pyvenv.cfg'
    $pythonLine = @(Get-Content -LiteralPath $venvConfig -Encoding UTF8 |
        Where-Object { $_.StartsWith('executable = ') })
    if ($pythonLine.Count -ne 1) {
        throw 'Piper virtual environment does not identify a unique base Python interpreter'
    }
    $piperBasePython = $pythonLine[0].Substring('executable = '.Length)
    if (-not (Test-Path -LiteralPath $piperBasePython)) {
        throw 'Piper base Python interpreter is unavailable'
    }
    if (-not (Test-Path -LiteralPath $piperPython) -or
        ($Piper -and -not (Test-Path -LiteralPath $piperModel))) {
        throw 'Local voice setup returned without validated runtime and model files'
    }
    $privateVoicePort = Get-AvailablePrivateVoicePort
}

# Staging cache is never a running executable. The two run slots permit
# an atomic candidate handoff without overwriting a live Windows EXE.
$legacyExe = Join-Path $repoRoot 'target\debug\vpr-owner-lab.exe'
$liveRoot = Join-Path $repoRoot 'target\rt0-live'
$slotAExe = Join-Path $liveRoot 'a\vpr-owner-lab.exe'
$slotBExe = Join-Path $liveRoot 'b\vpr-owner-lab.exe'
$oldExe = Get-VerifiedExistingOwnerLabExe -LegacyExe $legacyExe -SlotAExe $slotAExe -SlotBExe $slotBExe
$exe = if ($oldExe -and $oldExe -eq [IO.Path]::GetFullPath($slotAExe)) { $slotBExe } else { $slotAExe }
Assert-SafeToRestart

$piperProcess = $null
$piperListenerPid = 0
$labProcess = $null
$labReady = $false
try {
    Push-Location $repoRoot
    try {
        if (-not $Rt0Evidence) {
            git switch main
            if ($LASTEXITCODE -ne 0) { throw 'git switch main failed' }
            git pull --ff-only
            if ($LASTEXITCODE -ne 0) { throw 'git pull --ff-only failed' }
        }
        $previousCargoTargetDir = $env:CARGO_TARGET_DIR
        try {
            $env:CARGO_TARGET_DIR = Join-Path $repoRoot 'target\rt0-staging'
            cargo build --locked -p vpr-owner-lab --bins
            if ($LASTEXITCODE -ne 0) { throw 'Staging build failed before stopping the old Owner Lab' }
        } finally {
            if ($null -eq $previousCargoTargetDir) { Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue }
            else { $env:CARGO_TARGET_DIR = $previousCargoTargetDir }
        }
    } finally { Pop-Location }

    $builtDir = Join-Path $repoRoot 'target\rt0-staging\debug'
    $builtExe = Join-Path $builtDir 'vpr-owner-lab.exe'
    $credentialsExe = Join-Path $builtDir 'vpr-provider-credentials.exe'
    if (-not (Test-Path -LiteralPath $builtExe)) { throw "Missing staged Owner Lab: $builtExe" }
    if (-not (Test-Path -LiteralPath $credentialsExe)) { throw "Missing staged provider probe: $credentialsExe" }
    $null = New-Item -ItemType Directory -Force -Path (Split-Path -Path $exe -Parent)
    Copy-Item -LiteralPath $builtExe -Destination $exe -Force

    Clear-ProviderEnvironmentOverrides
    if ($VoiceProfile) { $env:VPR_OWNER_LAB_VOICE_PROFILE_PATH = $resolvedVoiceProfile }
    if ($Piper -or $SystemVoice) {
        # The D-ID Expressive credential probe must see the actual selected
        # server-owned TTS path BEFORE evaluating its Echo safety gate.
        # Clear stale fallback/provider overrides without changing legacy mode.
        foreach ($name in @(
            'VPR_OWNER_LAB_TTS_ALT_PROVIDER', 'VPR_OWNER_LAB_TTS_ALT_ENDPOINT',
            'VPR_OWNER_LAB_TTS_ALT_API_KEY', 'VPR_OWNER_LAB_TTS_ALT_MODEL',
            'VPR_OWNER_LAB_TTS_ALT_VOICE', 'VPR_DID_ECHO_ENABLED'
        )) {
            Remove-Item -Path "Env:$name" -ErrorAction SilentlyContinue
        }
        if ($Piper) { $env:VPR_PIPER_MODEL = $piperModel }
        else { Remove-Item Env:VPR_PIPER_MODEL -ErrorAction SilentlyContinue }
        $env:VPR_METROTRANCE_TTS_TOKEN = New-PiperPrivateToken
        $env:VPR_METROTRANCE_TTS_PORT = [string]$privateVoicePort
        $env:VPR_OWNER_LAB_TTS_PROVIDER = if ($SystemVoice) { 'windows-sapi' } else { 'piper' }
        $env:VPR_OWNER_LAB_TTS_ENDPOINT = "http://127.0.0.1:$privateVoicePort/v1/audio/speech"
        $env:VPR_OWNER_LAB_TTS_API_KEY = $env:VPR_METROTRANCE_TTS_TOKEN
        $env:VPR_OWNER_LAB_TTS_MODEL = if ($SystemVoice) { 'windows-sapi' } else { 'piper' }
        $env:VPR_OWNER_LAB_TTS_VOICE = 'local'
        $env:VPR_DID_ECHO_PYTHON = $piperPython
    }
    & $credentialsExe probe-avatar
    if ($LASTEXITCODE -ne 0) {
        throw 'Avatar provider preflight failed; Owner Lab was not started'
    }

    $rt0EvidenceValue = if ($Rt0Evidence) { 'true' } else { 'false' }
    if ($Piper -or $SystemVoice) {
        $piperBridge = Join-Path $repoRoot $(if ($SystemVoice) { 'scripts\windows_sapi_tts_bridge.py' } else { 'scripts\piper_tts_bridge.py' })
        $piperProcess = Start-Process -FilePath $piperPython -ArgumentList ('"' + $piperBridge + '"') -WorkingDirectory $repoRoot -PassThru
        Wait-PiperReady -Process $piperProcess
    }
    # Prove that the fully configured Owner Lab can actually start and answer
    # the canonical HTTP APIs BEFORE taking down the old listener.
    $canaryPort = Get-AvailableCanaryPort -PrimaryPort $Port
    $canaryProcess = $null
    try {
        $env:VPR_OWNER_LAB_PORT = [string]$canaryPort
        $env:VPR_OWNER_LAB_ALLOW_EGRESS = 'true'
        $env:VPR_OWNER_LAB_RT0_EVIDENCE = $rt0EvidenceValue
        $canaryProcess = Start-Process -FilePath $exe -ArgumentList '--allow-egress' -WorkingDirectory $repoRoot -PassThru
        Wait-CanaryLabReady -Process $canaryProcess -ListenPort $canaryPort -ExpectedExe $exe -StrictEvidence ([bool]$Rt0Evidence)
        Write-Host "Candidate Owner Lab verified on private port $canaryPort; no previous session was stopped."
    } finally {
        try {
            if ($null -ne $canaryProcess) {
                Stop-CanaryLab -Process $canaryProcess -ListenPort $canaryPort
            }
        } finally {
            $env:VPR_OWNER_LAB_PORT = [string]$Port
        }
    }

    # Final handoff is permitted ONLY after staged binary, provider probe,
    # authenticated private TTS and full Owner Lab HTTP readiness have passed.
    if ($oldExe) {
        Assert-SafeToRestart
        Stop-PortListener -ExpectedExe $oldExe
    }
    if ($Piper -or $SystemVoice) {
        $env:VPR_OWNER_LAB_ALLOW_EGRESS = 'true'
        $env:VPR_OWNER_LAB_PORT = [string]$Port
        $env:VPR_OWNER_LAB_RT0_EVIDENCE = $rt0EvidenceValue
        $labProcess = Start-Process -FilePath $exe -ArgumentList '--allow-egress' -WorkingDirectory $repoRoot -PassThru
    } else {
        $cmd = "set `"VPR_OWNER_LAB_ALLOW_EGRESS=true`" && set `"VPR_OWNER_LAB_PORT=$Port`" && set `"VPR_OWNER_LAB_RT0_EVIDENCE=$rt0EvidenceValue`" && `"$exe`" --allow-egress"
        Start-Process -FilePath 'cmd.exe' -ArgumentList '/k', $cmd -WorkingDirectory $repoRoot | Out-Null
    }
    Wait-LabUp

    $listenerPid = Assert-ExpectedListener -ExpectedExe $exe
    $bootstrap = Get-Bootstrap
    $status = Get-LabStatus
    if (-not $bootstrap.egress_enabled -or -not $status.egress_enabled) {
        throw 'New Owner Lab started, but backend egress is still disabled'
    }
    if ($Rt0Evidence -and -not $bootstrap.rt0_evidence_mode) {
        throw 'RT0 evidence launcher requested strict mode, but backend did not acknowledge it'
    }
    if ($status.conversation_readiness -ne 'text_and_voice') {
        throw "Provider profile is incomplete: conversation_readiness=$($status.conversation_readiness)"
    }

    $status = Get-LabStatus
    $labReady = $true
    Write-Host "Owner Lab ready: pid=$listenerPid port=$Port egress=$($status.egress_enabled), conversation=$($status.conversation_readiness), persona=$($status.owner_context_state)."
    if (-not $NoBrowser) {
        if ($Rt0Evidence) {
            Start-Process -FilePath $evidenceBrowser -ArgumentList '-new-window', $baseUrl | Out-Null
        } else {
            Start-Process $baseUrl
        }
    }
    if ($Piper -or $SystemVoice) {
        Write-Host 'Private voice-managed Owner Lab is running. Keep this window open until you finish.'
        $labProcess.WaitForExit()
    }
} finally {
    # Stop only the child started by this exact process handle; never a PID
    # recovered from an untrusted file, nor a service owning somebody else's port.
    if ($null -ne $piperProcess) {
        try {
            if ($piperListenerPid -gt 0 -and $piperListenerPid -ne $piperProcess.Id -and
                (Test-VerifiedPiperListener -Process $piperProcess -OwnerPid $piperListenerPid -VenvPython $piperPython -BasePython $piperBasePython -BridgeScript $piperBridge)) {
                # Retain an OS process handle before the final identity check,
                # so recycling the integer PID cannot terminate another app.
                $ownedChild = Get-Process -Id $piperListenerPid -ErrorAction SilentlyContinue
                if ($null -ne $ownedChild) {
                    try {
                        $null = $ownedChild.Handle
                        if ($ownedChild.Path -eq $piperBasePython -and
                            (Test-VerifiedPiperListener -Process $piperProcess -OwnerPid $ownedChild.Id -VenvPython $piperPython -BasePython $piperBasePython -BridgeScript $piperBridge)) {
                            $ownedChild.Kill()
                        }
                    } finally { $ownedChild.Close() }
                }
            }
            if (-not $piperProcess.HasExited) { $piperProcess.Kill() }
        } finally { $piperProcess.Close() }
    }
    if ($null -ne $labProcess) {
        try {
            if (-not $labReady -and -not $labProcess.HasExited) {
                $labProcess.Kill()
            }
        } finally { $labProcess.Close() }
    }
    foreach ($secretVar in @('VPR_METROTRANCE_TTS_TOKEN', 'VPR_OWNER_LAB_TTS_API_KEY')) {
        Remove-Item -Path "Env:$secretVar" -ErrorAction SilentlyContinue
    }
}
 -or
        [int]$voiceConfig.version -lt 1) {
        throw 'INVALID_VOICE_PROFILE_IDENTITY'
    }
    if ($Rt0Evidence -and $voiceConfig.status -eq 'CANDIDATE') {
        throw 'CANDIDATE_VOICE_NOT_ALLOWED_FOR_RT0_EVIDENCE'
    }
    if ($Rt0Evidence -and $voiceConfig.status -notin @('INTERIM','ACCEPTED')) {
        throw 'RT0_EVIDENCE_VOICE_STATUS_INVALID'
    }
    foreach ($pair in @(
        @('VPR_OWNER_LAB_TTS_PROVIDER', 'provider'),
        @('VPR_OWNER_LAB_TTS_MODEL', 'model'),
        @('VPR_OWNER_LAB_TTS_VOICE', 'voice')
    )) {
        $actual = [Environment]::GetEnvironmentVariable($pair[0], 'Process')
        $expected = [string]$voiceConfig.($pair[1])
        if ($actual -cne $expected) { throw "VOICE_PROFILE_ENV_CONFLICT: $($pair[0])" }
    }
    Write-Host "Voice profile: $($voiceConfig.profile_id) v$($voiceConfig.version) $($voiceConfig.status)"
}
if ($Rt0Evidence) {
    if ($env:VPR_OWNER_LAB_TTS_ALT_PROVIDER) { throw 'RT0_EVIDENCE_TTS_FALLBACK_FORBIDDEN' }
    if (($Piper -or $SystemVoice -or $env:VPR_OWNER_LAB_TTS_PROVIDER) -and -not $VoiceProfile) {
        throw 'RT0_EVIDENCE_VOICE_PROFILE_REQUIRED'
    }
}
if (($Piper -or $SystemVoice) -and $env:VPR_OWNER_LAB_TTS_PROVIDER -and
    $env:VPR_OWNER_LAB_TTS_PROVIDER -notin @('piper','windows-sapi')) {
    throw 'choose one voice source: configured cloud TTS cannot be overwritten'
}
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
    param([Parameter(Mandatory = $true)][string]$ExpectedExe)

    $listeners = @(Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue)
    if ($listeners.Count -eq 0) {
        return
    }

    # Never kill an unrelated service just because it occupies Owner Lab's port.
    # This also rejects ambiguous/multiple listeners and unknown executable paths.
    $verifiedProcess = Assert-ExpectedListener -ExpectedExe $ExpectedExe -PassThruProcess
    try {
        if ($verifiedProcess.Id -eq $PID) {
            throw "Refusing to stop the current PowerShell process"
        }
        Write-Host "Stopping verified stale Owner Lab listener PID $($verifiedProcess.Id) on port $Port."
        # The acquired process Handle pins its Windows process object across
        # verification and Kill; a recycled numeric PID must never be resolved.
        $verifiedProcess.Kill()
    } finally {
        $verifiedProcess.Close()
    }
    Wait-LabDown
}

function Assert-ExpectedListener {
    param(
        [Parameter(Mandatory = $true)][string]$ExpectedExe,
        [int]$ListenPort = $Port,
        [switch]$PassThruProcess
    )

    $listeners = @(Get-NetTCPConnection -LocalPort $ListenPort -State Listen -ErrorAction SilentlyContinue)
    if ($listeners.Count -ne 1) {
        throw "Expected exactly one listener on port $ListenPort, found $($listeners.Count)"
    }

    $listenerPid = $listeners[0].OwningProcess
    $process = Get-Process -Id $listenerPid -ErrorAction Stop
    $handedOff = $false
    try {
        # Acquire a real process handle BEFORE reading its executable identity.
        # Retaining that handle prevents Windows from recycling the PID while
        # the checked process is being stopped.
        $null = $process.Handle
        $actualExe = $process.Path
        if ([string]::IsNullOrWhiteSpace($actualExe)) {
            throw "Could not resolve executable for listener PID $listenerPid"
        }

        $expectedPath = [System.IO.Path]::GetFullPath($ExpectedExe)
        $actualPath = [System.IO.Path]::GetFullPath($actualExe)
        if (-not [string]::Equals($expectedPath, $actualPath, [System.StringComparison]::OrdinalIgnoreCase)) {
            throw "Port $ListenPort is owned by unexpected process: $actualPath"
        }
        if ($PassThruProcess) {
            $handedOff = $true
            return $process
        }
        return $listenerPid
    } finally {
        if (-not $handedOff) {
            $process.Close()
        }
    }
}

function Get-VerifiedExistingOwnerLabExe {
    param([string]$LegacyExe, [string]$SlotAExe, [string]$SlotBExe)
    $listeners = @(Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue)
    if ($listeners.Count -eq 0) { return $null }
    if ($listeners.Count -ne 1) { throw "Ambiguous listener on $Port" }
    $process = Get-Process -Id $listeners[0].OwningProcess -ErrorAction Stop
    try {
        $null = $process.Handle
        $actual = $process.Path
        if ([string]::IsNullOrWhiteSpace($actual)) { throw 'Unverifiable Owner Lab process' }
        foreach ($candidate in @($LegacyExe, $SlotAExe, $SlotBExe)) {
            $expected = [System.IO.Path]::GetFullPath($candidate)
            if ([string]::Equals([System.IO.Path]::GetFullPath($actual), $expected, [System.StringComparison]::OrdinalIgnoreCase)) { return $expected }
        }
        throw "Foreign listener on Owner Lab port: $actual"
    } finally { $process.Close() }
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
        'VPR_OWNER_LAB_LLM_MODEL',
        'VPR_OWNER_LAB_RT0_EVIDENCE'
    )
    foreach ($name in $names) {
        Remove-Item -Path "Env:$name" -ErrorAction SilentlyContinue
    }
    Write-Host "Cleared inherited provider env overrides; canonical Windows credential profile is authoritative."
}

function New-PiperPrivateToken {
    $bytes = New-Object byte[] 32
    $rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
    try { $rng.GetBytes($bytes) } finally { $rng.Dispose() }
    return [Convert]::ToBase64String($bytes)
}

function Get-AvailableCanaryPort {
    param([int]$PrimaryPort = $Port)
    if ($PrimaryPort -lt 1 -or $PrimaryPort -gt 65503) {
        throw 'Owner Lab primary port cannot support a private canary port'
    }
    foreach ($candidate in ($PrimaryPort + 1)..($PrimaryPort + 32)) {
        if (@(Get-NetTCPConnection -LocalPort $candidate -State Listen -ErrorAction SilentlyContinue).Count -eq 0) {
            return [int]$candidate
        }
    }
    throw 'No free canary port; refusing to replace an existing Owner Lab'
}

function Assert-CanaryReadiness {
    param(
        [Parameter(Mandatory = $true)][object]$Bootstrap,
        [Parameter(Mandatory = $true)][object]$Status,
        [bool]$StrictEvidence
    )
    if (-not $Bootstrap.egress_enabled -or -not $Status.egress_enabled) {
        throw 'Staged Owner Lab did not acknowledge provider egress'
    }
    if ($StrictEvidence -and -not $Bootstrap.rt0_evidence_mode) {
        throw 'Staged Owner Lab did not acknowledge exact-candidate RT0 evidence mode'
    }
    if ($Status.conversation_readiness -ne 'text_and_voice') {
        throw "Staged Owner Lab is not text-and-voice ready: $($Status.conversation_readiness)"
    }
    if ($Status.session_state -notin @('none', 'closed')) {
        throw 'Staged Owner Lab must have no active session before handoff'
    }
}

function Wait-CanaryLabReady {
    param(
        [Parameter(Mandatory = $true)][object]$Process,
        [Parameter(Mandatory = $true)][int]$ListenPort,
        [Parameter(Mandatory = $true)][string]$ExpectedExe,
        [bool]$StrictEvidence,
        [int]$TimeoutSeconds = 45
    )
    $canaryUrl = "http://127.0.0.1:$ListenPort"
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $lastReadinessError = 'server never accepted the readiness requests'
    while ([DateTime]::UtcNow -lt $deadline) {
        if ($Process.HasExited) {
            throw 'Staged Owner Lab process exited before startup readiness'
        }
        $listeners = @(Get-NetTCPConnection -LocalPort $ListenPort -State Listen -ErrorAction SilentlyContinue)
        if ($listeners.Count -gt 0) {
            # Fail immediately on an unknown/ambiguous listener; NEVER probe it.
            $actualPid = Assert-ExpectedListener -ExpectedExe $ExpectedExe -ListenPort $ListenPort
            if ($actualPid -ne $Process.Id) {
                throw 'Private canary port is owned by a different process'
            }
            try {
                $bootstrap = Invoke-RestMethod -Method Get -Uri "$canaryUrl/api/bootstrap" -TimeoutSec 2
                $status = Invoke-RestMethod -Method Get -Uri "$canaryUrl/api/status" -TimeoutSec 2
                Assert-CanaryReadiness -Bootstrap $bootstrap -Status $status -StrictEvidence $StrictEvidence
                # A listening server without its embedded browser UI is not
                # a usable Owner Lab. Check both frontend entrypoints.
                $page = Invoke-WebRequest -UseBasicParsing -Method Get -Uri "$canaryUrl/" -TimeoutSec 2
                $bundle = Invoke-WebRequest -UseBasicParsing -Method Get -Uri "$canaryUrl/app.js" -TimeoutSec 2
                if ($page.StatusCode -ne 200 -or $bundle.StatusCode -ne 200 -or
                    [string]::IsNullOrWhiteSpace([string]$page.Content) -or
                    [string]::IsNullOrWhiteSpace([string]$bundle.Content)) {
                    throw 'Staged Owner Lab browser assets unavailable'
                }
                return
            } catch {
                $lastReadinessError = $_.Exception.Message
            }
        }
        Start-Sleep -Milliseconds 500
    }
    throw "Staged Owner Lab failed readiness before cutover: $lastReadinessError"
}

function Stop-CanaryLab {
    param(
        [Parameter(Mandatory = $true)][object]$Process,
        [Parameter(Mandatory = $true)][int]$ListenPort
    )
    # The Process object originates from Start-Process -PassThru; never
    # terminate a numeric PID looked up from a port or an untrusted file.
    try {
        if (-not $Process.HasExited) {
            $Process.Kill()
        }
        if (-not $Process.WaitForExit(15000)) {
            throw 'Staged Owner Lab failed to stop before the primary cutover'
        }
        # TCP teardown is asynchronous on Windows; allow a short bounded
        # release window rather than falsely failing an otherwise safe stop.
        $deadline = [DateTime]::UtcNow.AddSeconds(5)
        while ([DateTime]::UtcNow -lt $deadline) {
            if (@(Get-NetTCPConnection -LocalPort $ListenPort -State Listen -ErrorAction SilentlyContinue).Count -eq 0) {
                return
            }
            Start-Sleep -Milliseconds 125
        }
        throw 'Canary port still has a listener after process termination'
    } finally {
        $Process.Close()
    }
}

function Get-AvailablePrivateVoicePort {
    foreach ($candidate in 8897..8927) {
        if (@(Get-NetTCPConnection -LocalPort $candidate -State Listen -ErrorAction SilentlyContinue).Count -eq 0) { return [int]$candidate }
    }
    throw 'No free private voice port; existing Owner Lab was preserved'
}

function Test-VerifiedPiperListener {
    param(
        [Parameter(Mandatory = $true)][System.Diagnostics.Process]$Process,
        [Parameter(Mandatory = $true)][int]$OwnerPid,
        [Parameter(Mandatory = $true)][string]$VenvPython,
        [Parameter(Mandatory = $true)][string]$BasePython,
        [Parameter(Mandatory = $true)][string]$BridgeScript
    )
    if ($Process.HasExited) { return $false }
    # On Windows, a Python venv python.exe may be a redirector which creates
    # a child process running the base interpreter. Bind either case to the
    # original live process handle AND its expected executable / bridge path.
    $owner = Get-CimInstance Win32_Process -Filter "ProcessId = $OwnerPid" -ErrorAction SilentlyContinue
    if ($null -eq $owner) { return $false }
    $compare = [StringComparison]::OrdinalIgnoreCase
    if ($OwnerPid -eq $Process.Id) {
        return $owner.ExecutablePath -eq $VenvPython -and
            $owner.CommandLine -and $owner.CommandLine.IndexOf($BridgeScript, $compare) -ge 0
    }
    if ($owner.ParentProcessId -ne $Process.Id -or $owner.ExecutablePath -ne $BasePython -or
        -not $owner.CommandLine -or $owner.CommandLine.IndexOf($BridgeScript, $compare) -lt 0) {
        return $false
    }
    $parent = Get-CimInstance Win32_Process -Filter "ProcessId = $($Process.Id)" -ErrorAction SilentlyContinue
    return $null -ne $parent -and $parent.ExecutablePath -eq $VenvPython -and
        $parent.CommandLine -and $parent.CommandLine.IndexOf($BridgeScript, $compare) -ge 0
}

function Wait-PiperReady {
    param([Parameter(Mandatory = $true)][System.Diagnostics.Process]$Process)
    $deadline = [DateTime]::UtcNow.AddSeconds(75)
    $headers = @{ Authorization = "Bearer $env:VPR_METROTRANCE_TTS_TOKEN" }
    $selectedModel = if ($SystemVoice) { 'windows-sapi' } else { 'piper' }
    $body = '{"input":"Проверка голоса.","model":"' + $selectedModel + '","voice":"local","response_format":"wav"}'
    while ([DateTime]::UtcNow -lt $deadline) {
        if ($Process.HasExited) {
            throw 'Piper exited before its authenticated speech endpoint became ready'
        }
        # Check the actual Windows listener owner BEFORE sending the bearer
        # token. A foreign process racing the bind must never receive secrets.
        $listeners = @(Get-NetTCPConnection -LocalPort $privateVoicePort -State Listen -ErrorAction SilentlyContinue)
        if ($listeners.Count -gt 0) {
            if ($listeners.Count -ne 1 -or $listeners[0].LocalAddress -ne '127.0.0.1' -or
                -not (Test-VerifiedPiperListener -Process $Process -OwnerPid $listeners[0].OwningProcess -VenvPython $piperPython -BasePython $piperBasePython -BridgeScript $piperBridge)) {
                throw 'Piper port owner is not the verified spawned Python interpreter or its direct child'
            }
            $script:piperListenerPid = [int]$listeners[0].OwningProcess
        } else {
            Start-Sleep -Milliseconds 750
            continue
        }
        try {
            $result = Invoke-WebRequest -UseBasicParsing -Method Post -Uri ("http://127.0.0.1:$privateVoicePort/v1/audio/speech") -Headers $headers -ContentType 'application/json; charset=utf-8' -Body ([Text.Encoding]::UTF8.GetBytes($body)) -TimeoutSec 10
            if ($result.StatusCode -eq 200 -and $result.Headers['Content-Type'] -like 'audio/wav*') {
                return
            }
        } catch {
            # Connection can be refused while the model loads.
        }
        Start-Sleep -Milliseconds 750
    }
    throw 'Piper did not pass authenticated local speech warmup'
}

function Resolve-Rt0EvidenceBrowser {
    $programFilesX86 = [Environment]::GetFolderPath('ProgramFilesX86')
    $candidates = @()
    if ($env:ProgramFiles) {
        $candidates += Join-Path $env:ProgramFiles 'Mozilla Firefox\firefox.exe'
    }
    if ($programFilesX86) {
        $candidates += Join-Path $programFilesX86 'Mozilla Firefox\firefox.exe'
    }
    $candidates = @($candidates | Where-Object { Test-Path -LiteralPath $_ })

    $firefox = $candidates | Select-Object -First 1
    if (-not $firefox) {
        throw @"
RT0 exact-candidate A/V-sync evidence uses Firefox 142 or newer as the canonical evidence browser.
Strict RT0 A/V-sync evidence requires RTCInboundRtpStreamStats.estimatedPlayoutTimestamp.
HTML media-element currentTime is diagnostic-only and cannot qualify release evidence. Install Firefox 142 or newer, then rerun:
  powershell -NoProfile -ExecutionPolicy Bypass -File scripts\windows-owner-lab-restart.ps1 -Rt0Evidence
The Owner Lab backend was not opened in a non-evidence browser.
"@
    }

    $version = (Get-Item -LiteralPath $firefox).VersionInfo.ProductVersion
    $majorText = ($version -split '\.')[0]
    $major = 0
    if (-not [int]::TryParse($majorText, [ref]$major) -or $major -lt 142) {
        throw "RT0 exact-candidate evidence requires the canonical Firefox 142+ browser; found Firefox $version at $firefox"
    }

    Write-Host "RT0 evidence browser: Firefox $version ($firefox)"
    return $firefox
}

function Get-Rt0GitCandidate {
    param([Parameter(Mandatory = $true)][string]$RepositoryRoot)

    # ZIP/source-archive downloads lack .git metadata and cannot provide
    # verifiable exact-candidate provenance for release-evidence collection.
    if (-not (Test-Path -LiteralPath (Join-Path $RepositoryRoot '.git'))) {
        throw "RT0 evidence requires a Git clone, not a GitHub ZIP download. In CMD: git clone https://github.com/mailsvb2-bot/Virtual-Persona-Runtime.git ; then run this launcher from the cloned folder. No Owner Lab listener was stopped."
    }

    $candidateOutput = @(git -C $RepositoryRoot rev-parse --verify HEAD)
    if ($LASTEXITCODE -ne 0 -or $candidateOutput.Count -ne 1 -or $candidateOutput[0] -notmatch '^[0-9a-f]{40}$') {
        throw 'RT0 evidence requires a valid exact Git candidate; no Owner Lab listener was stopped'
    }
    return [string]$candidateOutput[0]
}

$evidenceBrowser = $null
if ($Rt0Evidence -and -not $NoBrowser) {
    $evidenceBrowser = Resolve-Rt0EvidenceBrowser
}

if ($Rt0Evidence) {
    Push-Location $repoRoot
    try {
        $candidate = Get-Rt0GitCandidate -RepositoryRoot $repoRoot
        $dirty = @(git status --porcelain --untracked-files=all)
        if ($LASTEXITCODE -ne 0 -or $dirty.Count -gt 0) {
            throw 'RT0 evidence requires a clean worktree; no listener was stopped'
        }
        Write-Host "Frozen RT0 candidate: $candidate (no Git switch or pull)"
    } finally {
        Pop-Location
    }
}


# Explicit opt-in download happens before stopping any existing session.
$piperPython = $null
$piperModel = $null
$privateVoicePort = 0
if ($Piper -or $SystemVoice) {
    if (-not $env:LOCALAPPDATA) {
        throw 'LocalAppData is required for the private Piper runtime'
    }
    $piperRoot = if ($SystemVoice) {
        Join-Path $env:LOCALAPPDATA 'Virtual-Persona-Runtime\Sapi'
    } else {
        Join-Path $env:LOCALAPPDATA 'Virtual-Persona-Runtime\Piper'
    }
    $preparer = Join-Path $repoRoot $(if ($SystemVoice) { 'scripts\windows_sapi_prepare.py' } else { 'scripts\windows_piper_prepare.py' })
    Write-Host $(if ($SystemVoice) { 'Preparing native Russian female voice and private Echo...' } else { 'Preparing small Russian Piper voice; only first use needs a download...' })
    & py -3.11 $preparer --data-dir $piperRoot
    if ($LASTEXITCODE -ne 0) {
        throw 'Piper setup failed before any Owner Lab session was stopped'
    }
    $piperPython = Join-Path $piperRoot 'venv\Scripts\python.exe'
    $piperModel = Join-Path $piperRoot 'voices\ru_RU-denis-medium.onnx'
    # Windows PowerShell 5.1 can corrupt non-ASCII paths emitted by native
    # Python stdout (e.g. C:\Users\Пользователь). Read Python's UTF-8 venv
    # configuration directly instead of guessing via terminal code pages.
    $venvConfig = Join-Path $piperRoot 'venv\pyvenv.cfg'
    $pythonLine = @(Get-Content -LiteralPath $venvConfig -Encoding UTF8 |
        Where-Object { $_.StartsWith('executable = ') })
    if ($pythonLine.Count -ne 1) {
        throw 'Piper virtual environment does not identify a unique base Python interpreter'
    }
    $piperBasePython = $pythonLine[0].Substring('executable = '.Length)
    if (-not (Test-Path -LiteralPath $piperBasePython)) {
        throw 'Piper base Python interpreter is unavailable'
    }
    if (-not (Test-Path -LiteralPath $piperPython) -or
        ($Piper -and -not (Test-Path -LiteralPath $piperModel))) {
        throw 'Local voice setup returned without validated runtime and model files'
    }
    $privateVoicePort = Get-AvailablePrivateVoicePort
}

# Staging cache is never a running executable. The two run slots permit
# an atomic candidate handoff without overwriting a live Windows EXE.
$legacyExe = Join-Path $repoRoot 'target\debug\vpr-owner-lab.exe'
$liveRoot = Join-Path $repoRoot 'target\rt0-live'
$slotAExe = Join-Path $liveRoot 'a\vpr-owner-lab.exe'
$slotBExe = Join-Path $liveRoot 'b\vpr-owner-lab.exe'
$oldExe = Get-VerifiedExistingOwnerLabExe -LegacyExe $legacyExe -SlotAExe $slotAExe -SlotBExe $slotBExe
$exe = if ($oldExe -and $oldExe -eq [IO.Path]::GetFullPath($slotAExe)) { $slotBExe } else { $slotAExe }
Assert-SafeToRestart

$piperProcess = $null
$piperListenerPid = 0
$labProcess = $null
$labReady = $false
try {
    Push-Location $repoRoot
    try {
        if (-not $Rt0Evidence) {
            git switch main
            if ($LASTEXITCODE -ne 0) { throw 'git switch main failed' }
            git pull --ff-only
            if ($LASTEXITCODE -ne 0) { throw 'git pull --ff-only failed' }
        }
        $previousCargoTargetDir = $env:CARGO_TARGET_DIR
        try {
            $env:CARGO_TARGET_DIR = Join-Path $repoRoot 'target\rt0-staging'
            cargo build --locked -p vpr-owner-lab --bins
            if ($LASTEXITCODE -ne 0) { throw 'Staging build failed before stopping the old Owner Lab' }
        } finally {
            if ($null -eq $previousCargoTargetDir) { Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue }
            else { $env:CARGO_TARGET_DIR = $previousCargoTargetDir }
        }
    } finally { Pop-Location }

    $builtDir = Join-Path $repoRoot 'target\rt0-staging\debug'
    $builtExe = Join-Path $builtDir 'vpr-owner-lab.exe'
    $credentialsExe = Join-Path $builtDir 'vpr-provider-credentials.exe'
    if (-not (Test-Path -LiteralPath $builtExe)) { throw "Missing staged Owner Lab: $builtExe" }
    if (-not (Test-Path -LiteralPath $credentialsExe)) { throw "Missing staged provider probe: $credentialsExe" }
    $null = New-Item -ItemType Directory -Force -Path (Split-Path -Path $exe -Parent)
    Copy-Item -LiteralPath $builtExe -Destination $exe -Force

    Clear-ProviderEnvironmentOverrides
    if ($Piper -or $SystemVoice) {
        # The D-ID Expressive credential probe must see the actual selected
        # server-owned TTS path BEFORE evaluating its Echo safety gate.
        # Clear stale fallback/provider overrides without changing legacy mode.
        foreach ($name in @(
            'VPR_OWNER_LAB_TTS_ALT_PROVIDER', 'VPR_OWNER_LAB_TTS_ALT_ENDPOINT',
            'VPR_OWNER_LAB_TTS_ALT_API_KEY', 'VPR_OWNER_LAB_TTS_ALT_MODEL',
            'VPR_OWNER_LAB_TTS_ALT_VOICE', 'VPR_DID_ECHO_ENABLED'
        )) {
            Remove-Item -Path "Env:$name" -ErrorAction SilentlyContinue
        }
        if ($Piper) { $env:VPR_PIPER_MODEL = $piperModel }
        else { Remove-Item Env:VPR_PIPER_MODEL -ErrorAction SilentlyContinue }
        $env:VPR_METROTRANCE_TTS_TOKEN = New-PiperPrivateToken
        $env:VPR_METROTRANCE_TTS_PORT = [string]$privateVoicePort
        $env:VPR_OWNER_LAB_TTS_PROVIDER = if ($SystemVoice) { 'windows-sapi' } else { 'piper' }
        $env:VPR_OWNER_LAB_TTS_ENDPOINT = "http://127.0.0.1:$privateVoicePort/v1/audio/speech"
        $env:VPR_OWNER_LAB_TTS_API_KEY = $env:VPR_METROTRANCE_TTS_TOKEN
        $env:VPR_OWNER_LAB_TTS_MODEL = if ($SystemVoice) { 'windows-sapi' } else { 'piper' }
        $env:VPR_OWNER_LAB_TTS_VOICE = 'local'
        $env:VPR_DID_ECHO_PYTHON = $piperPython
    }
    & $credentialsExe probe-avatar
    if ($LASTEXITCODE -ne 0) {
        throw 'Avatar provider preflight failed; Owner Lab was not started'
    }

    $rt0EvidenceValue = if ($Rt0Evidence) { 'true' } else { 'false' }
    if ($Piper -or $SystemVoice) {
        $piperBridge = Join-Path $repoRoot $(if ($SystemVoice) { 'scripts\windows_sapi_tts_bridge.py' } else { 'scripts\piper_tts_bridge.py' })
        $piperProcess = Start-Process -FilePath $piperPython -ArgumentList ('"' + $piperBridge + '"') -WorkingDirectory $repoRoot -PassThru
        Wait-PiperReady -Process $piperProcess
    }
    # Prove that the fully configured Owner Lab can actually start and answer
    # the canonical HTTP APIs BEFORE taking down the old listener.
    $canaryPort = Get-AvailableCanaryPort -PrimaryPort $Port
    $canaryProcess = $null
    try {
        $env:VPR_OWNER_LAB_PORT = [string]$canaryPort
        $env:VPR_OWNER_LAB_ALLOW_EGRESS = 'true'
        $env:VPR_OWNER_LAB_RT0_EVIDENCE = $rt0EvidenceValue
        $canaryProcess = Start-Process -FilePath $exe -ArgumentList '--allow-egress' -WorkingDirectory $repoRoot -PassThru
        Wait-CanaryLabReady -Process $canaryProcess -ListenPort $canaryPort -ExpectedExe $exe -StrictEvidence ([bool]$Rt0Evidence)
        Write-Host "Candidate Owner Lab verified on private port $canaryPort; no previous session was stopped."
    } finally {
        try {
            if ($null -ne $canaryProcess) {
                Stop-CanaryLab -Process $canaryProcess -ListenPort $canaryPort
            }
        } finally {
            $env:VPR_OWNER_LAB_PORT = [string]$Port
        }
    }

    # Final handoff is permitted ONLY after staged binary, provider probe,
    # authenticated private TTS and full Owner Lab HTTP readiness have passed.
    if ($oldExe) {
        Assert-SafeToRestart
        Stop-PortListener -ExpectedExe $oldExe
    }
    if ($Piper -or $SystemVoice) {
        $env:VPR_OWNER_LAB_ALLOW_EGRESS = 'true'
        $env:VPR_OWNER_LAB_PORT = [string]$Port
        $env:VPR_OWNER_LAB_RT0_EVIDENCE = $rt0EvidenceValue
        $labProcess = Start-Process -FilePath $exe -ArgumentList '--allow-egress' -WorkingDirectory $repoRoot -PassThru
    } else {
        $cmd = "set `"VPR_OWNER_LAB_ALLOW_EGRESS=true`" && set `"VPR_OWNER_LAB_PORT=$Port`" && set `"VPR_OWNER_LAB_RT0_EVIDENCE=$rt0EvidenceValue`" && `"$exe`" --allow-egress"
        Start-Process -FilePath 'cmd.exe' -ArgumentList '/k', $cmd -WorkingDirectory $repoRoot | Out-Null
    }
    Wait-LabUp

    $listenerPid = Assert-ExpectedListener -ExpectedExe $exe
    $bootstrap = Get-Bootstrap
    $status = Get-LabStatus
    if (-not $bootstrap.egress_enabled -or -not $status.egress_enabled) {
        throw 'New Owner Lab started, but backend egress is still disabled'
    }
    if ($Rt0Evidence -and -not $bootstrap.rt0_evidence_mode) {
        throw 'RT0 evidence launcher requested strict mode, but backend did not acknowledge it'
    }
    if ($status.conversation_readiness -ne 'text_and_voice') {
        throw "Provider profile is incomplete: conversation_readiness=$($status.conversation_readiness)"
    }

    $status = Get-LabStatus
    $labReady = $true
    Write-Host "Owner Lab ready: pid=$listenerPid port=$Port egress=$($status.egress_enabled), conversation=$($status.conversation_readiness), persona=$($status.owner_context_state)."
    if (-not $NoBrowser) {
        if ($Rt0Evidence) {
            Start-Process -FilePath $evidenceBrowser -ArgumentList '-new-window', $baseUrl | Out-Null
        } else {
            Start-Process $baseUrl
        }
    }
    if ($Piper -or $SystemVoice) {
        Write-Host 'Private voice-managed Owner Lab is running. Keep this window open until you finish.'
        $labProcess.WaitForExit()
    }
} finally {
    # Stop only the child started by this exact process handle; never a PID
    # recovered from an untrusted file, nor a service owning somebody else's port.
    if ($null -ne $piperProcess) {
        try {
            if ($piperListenerPid -gt 0 -and $piperListenerPid -ne $piperProcess.Id -and
                (Test-VerifiedPiperListener -Process $piperProcess -OwnerPid $piperListenerPid -VenvPython $piperPython -BasePython $piperBasePython -BridgeScript $piperBridge)) {
                # Retain an OS process handle before the final identity check,
                # so recycling the integer PID cannot terminate another app.
                $ownedChild = Get-Process -Id $piperListenerPid -ErrorAction SilentlyContinue
                if ($null -ne $ownedChild) {
                    try {
                        $null = $ownedChild.Handle
                        if ($ownedChild.Path -eq $piperBasePython -and
                            (Test-VerifiedPiperListener -Process $piperProcess -OwnerPid $ownedChild.Id -VenvPython $piperPython -BasePython $piperBasePython -BridgeScript $piperBridge)) {
                            $ownedChild.Kill()
                        }
                    } finally { $ownedChild.Close() }
                }
            }
            if (-not $piperProcess.HasExited) { $piperProcess.Kill() }
        } finally { $piperProcess.Close() }
    }
    if ($null -ne $labProcess) {
        try {
            if (-not $labReady -and -not $labProcess.HasExited) {
                $labProcess.Kill()
            }
        } finally { $labProcess.Close() }
    }
    foreach ($secretVar in @('VPR_METROTRANCE_TTS_TOKEN', 'VPR_OWNER_LAB_TTS_API_KEY')) {
        Remove-Item -Path "Env:$secretVar" -ErrorAction SilentlyContinue
    }
}
