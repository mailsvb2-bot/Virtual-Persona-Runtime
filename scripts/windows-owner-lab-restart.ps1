param(
    [int]$Port = 8787,
    [switch]$NoBrowser,
    [switch]$Rt0Evidence,
    [switch]$Piper
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
        [switch]$PassThruProcess
    )

    $listeners = @(Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue)
    if ($listeners.Count -ne 1) {
        throw "Expected exactly one listener on port $Port, found $($listeners.Count)"
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
            throw "Port $Port is owned by unexpected process: $actualPath"
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
    $body = '{"input":"Проверка голоса.","model":"piper","voice":"local","response_format":"wav"}'
    while ([DateTime]::UtcNow -lt $deadline) {
        if ($Process.HasExited) {
            throw 'Piper exited before its authenticated speech endpoint became ready'
        }
        # Check the actual Windows listener owner BEFORE sending the bearer
        # token. A foreign process racing the bind must never receive secrets.
        $listeners = @(Get-NetTCPConnection -LocalPort 8897 -State Listen -ErrorAction SilentlyContinue)
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
            $result = Invoke-WebRequest -UseBasicParsing -Method Post -Uri 'http://127.0.0.1:8897/v1/audio/speech' -Headers $headers -ContentType 'application/json; charset=utf-8' -Body ([Text.Encoding]::UTF8.GetBytes($body)) -TimeoutSec 10
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
if ($Piper) {
    if (-not $env:LOCALAPPDATA) {
        throw 'LocalAppData is required for the private Piper runtime'
    }
    $piperRoot = Join-Path $env:LOCALAPPDATA 'Virtual-Persona-Runtime\Piper'
    $preparer = Join-Path $repoRoot 'scripts\windows_piper_prepare.py'
    Write-Host 'Preparing small Russian Piper voice; only first use needs a download...'
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
    if (-not (Test-Path -LiteralPath $piperPython) -or -not (Test-Path -LiteralPath $piperModel)) {
        throw 'Piper setup returned without validated runtime and model files'
    }
    if (@(Get-NetTCPConnection -LocalPort 8897 -State Listen -ErrorAction SilentlyContinue).Count -gt 0) {
        throw 'Piper loopback port 8897 is occupied; refusing to use or terminate an unknown listener'
    }
}

$exe = Join-Path $repoRoot 'target\debug\vpr-owner-lab.exe'
Assert-SafeToRestart
Stop-PortListener -ExpectedExe $exe

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
        cargo build --locked -p vpr-owner-lab --bins
        if ($LASTEXITCODE -ne 0) { throw 'Owner Lab binaries build failed' }
    } finally {
        Pop-Location
    }

    $credentialsExe = Join-Path $repoRoot 'target\debug\vpr-provider-credentials.exe'
    if (-not (Test-Path -LiteralPath $exe)) {
        throw "Owner Lab executable was not produced: $exe"
    }
    if (-not (Test-Path -LiteralPath $credentialsExe)) {
        throw "Provider credential diagnostic was not produced: $credentialsExe"
    }

    Clear-ProviderEnvironmentOverrides
    if ($Piper) {
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
        $env:VPR_PIPER_MODEL = $piperModel
        $env:VPR_METROTRANCE_TTS_TOKEN = New-PiperPrivateToken
        $env:VPR_METROTRANCE_TTS_PORT = '8897'
        $env:VPR_OWNER_LAB_TTS_PROVIDER = 'piper'
        $env:VPR_OWNER_LAB_TTS_ENDPOINT = 'http://127.0.0.1:8897/v1/audio/speech'
        $env:VPR_OWNER_LAB_TTS_API_KEY = $env:VPR_METROTRANCE_TTS_TOKEN
        $env:VPR_OWNER_LAB_TTS_MODEL = 'piper'
        $env:VPR_OWNER_LAB_TTS_VOICE = 'local'
        $env:VPR_DID_ECHO_PYTHON = $piperPython
    }
    & $credentialsExe probe-avatar
    if ($LASTEXITCODE -ne 0) {
        throw 'Avatar provider preflight failed; Owner Lab was not started'
    }

    $rt0EvidenceValue = if ($Rt0Evidence) { 'true' } else { 'false' }
    if ($Piper) {
        $piperBridge = Join-Path $repoRoot 'scripts\piper_tts_bridge.py'
        $piperProcess = Start-Process -FilePath $piperPython -ArgumentList ('"' + $piperBridge + '"') -WorkingDirectory $repoRoot -PassThru
        Wait-PiperReady -Process $piperProcess
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
    if ($Piper) {
        Write-Host 'Piper-managed Owner Lab is running. Keep this window open until you finish.'
        $labProcess.WaitForExit()
    }
} finally {
    # Stop only the child started by this exact process handle; never a PID
    # recovered from an untrusted file, nor a service owning somebody else's port.
    if ($null -ne $piperProcess) {
        try {
            if ($piperListenerPid -gt 0 -and $piperListenerPid -ne $piperProcess.Id -and
                (Test-VerifiedPiperListener -Process $piperProcess -OwnerPid $piperListenerPid -VenvPython $piperPython -BasePython $piperBasePython -BridgeScript $piperBridge)) {
                Stop-Process -Id $piperListenerPid -ErrorAction SilentlyContinue
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
