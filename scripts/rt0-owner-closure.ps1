<#
.SYNOPSIS
  Owner-run RT0 closure for a clean exact Git candidate.
.DESCRIPTION
  Compare probes real Yandex and ProxyAPI, runs live Owner Lab and scores real exports.
  Exit collects real owner/visitor evidence. Finish evaluates reviewed private inputs.
  Never fabricates human, audio, provider or privacy evidence.
#>
param(
    [ValidateSet('Compare','Exit','Finish')][string]$Stage = 'Compare',
    [string]$EvidenceRoot,
    [ValidateSet('yandex','proxyapi-deepseek')][string[]]$Llms = @('yandex','proxyapi-deepseek'),
    [ValidateSet('yandex','proxyapi-deepseek')][string]$ExitLlm = 'yandex',
    [ValidateSet('ru_RU-irina-medium','ru_RU-denis-medium')][string]$PiperVoice = 'ru_RU-irina-medium',
    [int]$MinTurns = 20,
    [int]$MinStops = 5,
    [switch]$SkipSetLlm,
    [string]$PrivateInputDir
)
$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Set-Location -LiteralPath $repoRoot
if (& git status --porcelain) { throw 'RT0 evidence requires a clean Git worktree' }
if ($LASTEXITCODE -ne 0) { throw 'git status failed' }
$candidate = (& git rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or $candidate -notmatch '^[0-9a-f]{40}$') { throw 'Cannot resolve exact Git candidate' }
if (-not $EvidenceRoot) {
    if (-not $env:LOCALAPPDATA) { throw 'LocalAppData required (or -EvidenceRoot)' }
    $EvidenceRoot = Join-Path $env:LOCALAPPDATA ('Virtual-Persona-Runtime\rt0-evidence\' + $candidate)
}
$EvidenceRoot = [IO.Path]::GetFullPath($EvidenceRoot)
if ($EvidenceRoot.StartsWith($repoRoot.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) {
    throw 'RT0 evidence must be outside the Git worktree'
}
$null = New-Item -ItemType Directory -Force -Path $EvidenceRoot
$ev = Join-Path $EvidenceRoot 'exit'
$null = New-Item -ItemType Directory -Force -Path $ev
if (-not $PrivateInputDir) { $PrivateInputDir = Join-Path $EvidenceRoot 'private-input' }
$null = New-Item -ItemType Directory -Force -Path $PrivateInputDir
$summaryPath = Join-Path $EvidenceRoot 'closure-summary.json'
$summary = [ordered]@{ schema = 'vpr-rt0-owner-closure-0.1'; candidate_sha = $candidate; stages = [ordered]@{} }
if (Test-Path -LiteralPath $summaryPath) {
    $old = Get-Content -LiteralPath $summaryPath -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($old.candidate_sha -eq $candidate) {
        foreach ($item in $old.stages.PSObject.Properties) { $summary.stages[$item.Name] = $item.Value }
    }
}
function Save-Summary {
    ($summary | ConvertTo-Json -Depth 8) | Set-Content -LiteralPath $summaryPath -Encoding UTF8
}
function Invoke-Cargo {
    param([string]$Package,[string]$Bin,[string[]]$Arguments,[string]$StdoutFile,[int[]]$AllowedExit = @(0))
    # Interactive setup must show Cargo's first-build progress. Silence here
    # previously looked like a frozen console before any credential prompt.
    $args = @('run','--locked')
    if ($StdoutFile) { $args += '-q' }
    $args += @('-p',$Package)
    if ($Bin) { $args += @('--bin',$Bin) }
    $args += '--'
    $args += $Arguments
    if ($StdoutFile) {
        $temp = $StdoutFile + '.partial'
        & cargo @args | Out-File -LiteralPath $temp -Encoding UTF8
        $code = $LASTEXITCODE
        if ($AllowedExit -contains $code) { Move-Item -LiteralPath $temp -Destination $StdoutFile -Force }
        else { Remove-Item -LiteralPath $temp -ErrorAction SilentlyContinue }
    } else {
        & cargo @args
        $code = $LASTEXITCODE
    }
    return $code
}
function Read-SecretOnce([string]$Label) {
    $secure = Read-Host -AsSecureString -Prompt $Label
    $ptr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure)
    try { return [Runtime.InteropServices.Marshal]::PtrToStringBSTR($ptr) }
    finally { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($ptr) }
}
function Probe-Http {
    param([string]$Uri,[hashtable]$Headers,[hashtable]$Body)
    # Windows PowerShell 5.1 on older Windows often defaults to TLS 1.0.
    # Yandex AI Studio requires modern TLS; add TLS 1.2 without disabling
    # other negotiated protocols used elsewhere by the hosting process.
    [Net.ServicePointManager]::SecurityProtocol = (
        [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    )
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $step = 'json'
    try {
        $bytes = [Text.Encoding]::UTF8.GetBytes(($Body | ConvertTo-Json -Compress -Depth 6))
        $step = 'http'
        $r = Invoke-WebRequest -UseBasicParsing -Method Post -Uri $Uri -Headers $Headers -ContentType 'application/json' -Body $bytes -TimeoutSec 30
        $watch.Stop()
        return [ordered]@{
            status = [int]$r.StatusCode
            elapsed_ms = [int]$watch.ElapsedMilliseconds
            sse = ([string]$r.Content).Contains('data:')
            usage_chunk = ([string]$r.Content).Contains('"usage"')
        }
    } catch {
        $watch.Stop()
        $status = 0
        if ($_.Exception.Response) { $status = [int]$_.Exception.Response.StatusCode }
        # Diagnostic class only: never persist exception messages, URLs with
        # credentials, request headers, response bodies or the secret key.
        $exceptionType = $_.Exception.GetType().Name
        $webStatus = if ($_.Exception -is [Net.WebException]) { [string]$_.Exception.Status } else { 'not-web-exception' }
        $errorCode = if ($_.Exception -is [ArgumentException]) { [string]$_.Exception.ParamName } else { '' }
        $innerType = if ($_.Exception.InnerException) { $_.Exception.InnerException.GetType().Name } else { '' }
        return [ordered]@{ status = $status; elapsed_ms = [int]$watch.ElapsedMilliseconds; sse = $false; usage_chunk = $false; exception_type = $exceptionType; transport_status = $webStatus; failed_step = $step; argument_parameter = $errorCode; inner_exception_type = $innerType }
    }
}
function Probe-Llm {
    param([string]$Llm,[string]$Dir)
    $result = [ordered]@{ schema = 'vpr-rt0-llm-wire-probe-0.1'; provider = $Llm; candidate_sha = $candidate; variants = @() }
    $messages = @(@{role = 'user';content = 'Ответь одним словом: привет'})
    if ($Llm -eq 'yandex') {
        $folder = Read-Host 'Yandex folder ID (same as set-llm)'
        if ($folder -notmatch '^[a-z0-9]{1,64}$') { throw 'invalid Yandex folder ID' }
        $key = Read-SecretOnce 'Yandex key for one-off wire probe'
        # Fail before any HTTP request when the console received a placeholder,
        # an ID, or a malformed/pasted multiline secret. No secret contents logged.
        if ($key.Length -lt 20 -or $key -cnotmatch '^[\\x21-\\x7E]+$') {
            throw "Yandex API key was not received as a complete single-line token (length=$($key.Length)). Paste the full secret value, not a masked preview."
        }
        $uri = 'https://ai.api.cloud.yandex.net/v1/chat/completions'
        $model = "gpt://$folder/yandexgpt-5.1"
        $chosen = $null
        foreach ($auth in @('apikey','bearer')) {
            foreach ($projectHeader in @('openai-project','x-folder-id')) {
                $prefix = if ($auth -eq 'apikey') { 'Api-Key' } else { 'Bearer' }
                $hName = if ($projectHeader -eq 'openai-project') { 'OpenAI-Project' } else { 'x-folder-id' }
                $headers = @{ Authorization = "$prefix $key" }
                $headers[$hName] = $folder
                $r = Probe-Http -Uri $uri -Headers $headers -Body @{model=$model;messages=$messages;stream=$true;max_tokens=8}
                $r['auth'] = $auth; $r['project_header'] = $projectHeader
                $result.variants += $r
                if (-not $chosen -and $r.status -ge 200 -and $r.status -lt 300) {
                    $chosen = @{auth=$auth;project=$projectHeader;headers=$headers}
                }
            }
        }
        if (-not $chosen) {
            $probePath = Join-Path $Dir 'llm-wire-probe.json'
            ($result | ConvertTo-Json -Depth 7) | Set-Content -LiteralPath $probePath -Encoding UTF8
            $codes = @($result.variants | ForEach-Object { "$($_.auth)/$($_.project_header)=$($_.status)" }) -join ', '
            throw "Yandex wire probe rejected ($codes). Sanitized statuses saved to $probePath. Check API key scope, account permissions, billing, and model access."
        }
        $usage = Probe-Http -Uri $uri -Headers $chosen.headers -Body @{model=$model;messages=$messages;stream=$true;max_tokens=8;stream_options=@{include_usage=$true}}
        $usage['variant'] = 'stream_options.include_usage'
        $result.variants += $usage
        $usageFlag = if ($usage.status -ge 200 -and $usage.status -lt 300 -and $usage.usage_chunk) { 'on' } else { 'off' }
        $result['selected'] = @{auth_scheme=$chosen.auth;project_header=$chosen.project;stream_usage=$usageFlag}
        $result['model'] = $model
        $env:VPR_OWNER_LAB_LLM_PROJECT_ID = $folder
        $env:VPR_OWNER_LAB_LLM_AUTH_SCHEME = $chosen.auth
        $env:VPR_OWNER_LAB_LLM_PROJECT_HEADER = $chosen.project
        $env:VPR_OWNER_LAB_LLM_STREAM_USAGE = $usageFlag
    } else {
        $model = Read-Host 'ProxyAPI exact model ID from provider catalogue'
        $key = Read-SecretOnce 'ProxyAPI key for one-off wire probe'
        $uri = 'https://api.proxyapi.ru/v1/chat/completions'
        $headers = @{ Authorization = "Bearer $key" }
        $plain = Probe-Http -Uri $uri -Headers $headers -Body @{model=$model;messages=$messages;stream=$true;max_tokens=8}
        $usage = Probe-Http -Uri $uri -Headers $headers -Body @{model=$model;messages=$messages;stream=$true;max_tokens=8;stream_options=@{include_usage=$true}}
        $thinking = Probe-Http -Uri $uri -Headers $headers -Body @{model=$model;messages=$messages;stream=$true;max_tokens=8;thinking=@{type='disabled'};reasoning_effort='none'}
        $result.variants = @($plain,$usage,$thinking)
        if ($plain.status -lt 200 -or $plain.status -ge 300) { throw 'ProxyAPI plain streaming failed' }
        $usageFlag = if ($usage.status -ge 200 -and $usage.status -lt 300 -and $usage.usage_chunk) { 'on' } else { 'off' }
        $result['selected'] = @{auth_scheme='bearer';stream_usage=$usageFlag;thinking_fields_accepted=($thinking.status -ge 200 -and $thinking.status -lt 300)}
        $result['model'] = $model
        Remove-Item Env:VPR_OWNER_LAB_LLM_AUTH_SCHEME,Env:VPR_OWNER_LAB_LLM_PROJECT_HEADER,Env:VPR_OWNER_LAB_LLM_PROJECT_ID -ErrorAction SilentlyContinue
        $env:VPR_OWNER_LAB_LLM_STREAM_USAGE = $usageFlag
    }
    $key = $null
    [GC]::Collect()
    ($result | ConvertTo-Json -Depth 7) | Set-Content -LiteralPath (Join-Path $Dir 'llm-wire-probe.json') -Encoding UTF8
    return $result
}
function Launch-OwnerLab([string]$StateOut) {
    $env:VPR_OWNER_LAB_PROVIDER_STATE_OUT = $StateOut
    $launcher = Join-Path $repoRoot 'scripts\windows-owner-lab-restart.ps1'
    # The voice launcher deliberately supervises its subprocess until session exit.
    # Run it in a separate console so the acceptance driver can collect evidence.
    $arguments = '-NoProfile -ExecutionPolicy Bypass -File "' + $launcher + '" -Rt0Evidence -Piper -PiperVoice ' + $PiperVoice
    $process = Start-Process -FilePath 'powershell.exe' -ArgumentList $arguments -WorkingDirectory $repoRoot -PassThru
    $deadline = [DateTime]::UtcNow.AddMinutes(10)
    $ready = $false
    while ([DateTime]::UtcNow -lt $deadline) {
        if ($process.HasExited) {
            throw "Owner Lab launcher exited before readiness (exit code $($process.ExitCode)); inspect its console"
        }
        if (Test-Path -LiteralPath $StateOut -PathType Leaf) {
            try {
                $status = Invoke-RestMethod -Method Get -Uri 'http://127.0.0.1:8787/api/status' -TimeoutSec 2
                if ($status.session_state -in @('none','closed') -and
                    $status.conversation_readiness -eq 'text_and_voice' -and
                    $status.egress_enabled) {
                    $ready = $true
                    break
                }
            } catch {
                # Server and speech worker may still be starting.
            }
        }
        Start-Sleep -Milliseconds 500
    }
    if (-not $ready) { throw 'Owner Lab readiness or exact provider-state export did not complete' }
    Write-Host "Verified running Owner Lab and exact provider-state: $StateOut"
}
function Collect-Sessions {
    param([string]$Dir,[datetime]$Since,[string]$Role,[int]$MinVoice,[int]$MinStop)
    Write-Host "Please complete real $Role conversation in Owner Lab and download evidence snapshot."
    if ($MinVoice -gt 0) { Write-Host "At least $MinVoice completed Russian voice turns and $MinStop STOP observations required." }
    [void](Read-Host 'Press Enter after downloading snapshot into Downloads')
    $downloads = Join-Path $env:USERPROFILE 'Downloads'
    $found = @()
    foreach ($file in (Get-ChildItem -LiteralPath $downloads -Filter '*.json' -File | Where-Object { $_.LastWriteTime -ge $Since })) {
        try { $snap = Get-Content -LiteralPath $file.FullName -Raw -Encoding UTF8 | ConvertFrom-Json } catch { continue }
        if ($snap.schema_version -notlike 'rt0-owner-lab-session-evidence-*' -or $snap.candidate_sha -ne $candidate) { continue }
        $dest = Join-Path $Dir ("session-{0}-{1}.json" -f $Role,$snap.session_sequence)
        Copy-Item -LiteralPath $file.FullName -Destination $dest -Force
        $found += [pscustomobject]@{path=$dest; snapshot=$snap}
    }
    if ($found.Count -eq 0) { throw "No genuine $Role snapshot for $candidate" }
    $voiceCount = 0; $stopCount = 0
    foreach ($item in $found) {
        $voiceCount += @($item.snapshot.voice_attempts | Where-Object { $_.status -eq 'completed' }).Count
        $stopCount += @($item.snapshot.media_events | Where-Object { $_.kind -eq 'interruption_stopped' }).Count
    }
    if ($voiceCount -lt $MinVoice -or $stopCount -lt $MinStop) {
        throw "Insufficient real evidence: voice turns=$voiceCount/$MinVoice, STOP=$stopCount/$MinStop"
    }
    return $found
}
function State-Llm([string]$StatePath) {
    $state = Get-Content -LiteralPath $StatePath -Raw -Encoding UTF8 | ConvertFrom-Json
    $binding = @($state.providers | Where-Object { $_.role -eq 'llm' })
    if ($binding.Count -ne 1) { throw 'provider-state must bind one LLM' }
    return $binding[0]
}
function Set-Llm([string]$Llm) {
    if ($SkipSetLlm) { return }
    Write-Host "Configure $Llm. The provider is preselected; enter only the requested folder/model and hidden API key."
    if ((Invoke-Cargo -Package 'vpr-owner-lab' -Bin 'vpr-provider-credentials' -Arguments @('set-llm', $Llm)) -ne 0) { throw 'set-llm failed' }
}
if ($Stage -eq 'Compare') {
    $results = [ordered]@{}
    foreach ($llm in $Llms) {
        $dir = Join-Path $EvidenceRoot ('llm-compare\' + $llm)
        $null = New-Item -ItemType Directory -Force -Path $dir
        Set-Llm $llm
        $probe = Probe-Llm -Llm $llm -Dir $dir
        $statePath = Join-Path $dir 'provider-state.json'
        Remove-Item -LiteralPath $statePath -ErrorAction SilentlyContinue
        $since = Get-Date
        Launch-OwnerLab $statePath
        $binding = State-Llm $statePath
        if ($binding.provider -ne $llm) { throw "LLM mismatch: actual $($binding.provider), required $llm" }
        $sessions = Collect-Sessions -Dir $dir -Since $since -Role 'owner' -MinVoice $MinTurns -MinStop $MinStops
        $reportPath = Join-Path $dir 'latency-report.json'
        $args = @($llm,$binding.model_or_representation,$candidate,$statePath) + @($sessions | ForEach-Object { $_.path })
        $code = Invoke-Cargo -Package 'vpr-evaluation' -Bin 'vpr-rt0-llm-latency' -Arguments $args -StdoutFile $reportPath
        if ($code -ne 0) { throw "Latency evaluator failed for $llm" }
        $report = Get-Content -LiteralPath $reportPath -Raw -Encoding UTF8 | ConvertFrom-Json
        $results[$llm] = @{verdict=$report.verdict;model=$binding.model_or_representation;report=$reportPath}
    }
    $summary.stages['compare'] = $results
    Save-Summary
    Write-Host "Comparison completed; see $summaryPath"
    exit 0
}
if ($Stage -eq 'Exit') {
    Set-Llm $ExitLlm
    $inputs = @('probe.raw','reviewed-profile.json','owner.raw','visitor.raw') | ForEach-Object { Join-Path $PrivateInputDir $_ }
    foreach ($item in $inputs) { if (-not (Test-Path -LiteralPath $item)) { throw "Missing private input $item" } }
    if ((Invoke-Cargo -Package 'vpr-evaluation' -Bin 'vpr-rt0-evidence-prepare' -Arguments @($ev,$candidate)) -ne 0) { throw 'evidence-prepare failed' }
    $state = Join-Path $ev 'provider-state.json'
    $labState = Join-Path $ev 'owner-lab-provider-state.json'
    Remove-Item -LiteralPath $labState -ErrorAction SilentlyContinue
    $since = Get-Date
    Launch-OwnerLab $labState
    $env:VPR_OWNER_LAB_TTS_PROVIDER = 'piper'
    $env:VPR_OWNER_LAB_TTS_MODEL = 'piper'
    $env:VPR_OWNER_LAB_TTS_VOICE = $PiperVoice
    $env:VPR_OWNER_LAB_TTS_ENDPOINT = 'http://127.0.0.1:8897/v1/audio/speech'
    $env:VPR_OWNER_LAB_TTS_API_KEY = [Guid]::NewGuid().ToString('N')
    $env:VPR_OWNER_LAB_RT0_EVIDENCE = 'true'
    $profileName = if ($PiperVoice -eq 'ru_RU-denis-medium') { 'voice-profile.interim-male.json' } else { 'voice-profile.interim.json' }
    $env:VPR_OWNER_LAB_VOICE_PROFILE_PATH = Join-Path $env:LOCALAPPDATA ('Virtual-Persona-Runtime\voice-profiles\' + $profileName)
    if ((Invoke-Cargo -Package 'vpr-live-proof' -Arguments (@('doctor')+$inputs)) -ne 0) { throw 'doctor failed' }
    $bundle = Join-Path $ev 'candidate-bundle.json'
    $env:VPR_LIVE_PROOF_ALLOW_EGRESS = 'true'
    try {
        if ((Invoke-Cargo -Package 'vpr-live-proof' -Arguments (@('candidate-bundle')+$inputs+@($bundle))) -ne 0) { throw 'candidate-bundle failed' }
    } finally { Remove-Item Env:VPR_LIVE_PROOF_ALLOW_EGRESS -ErrorAction SilentlyContinue }
    if ((Invoke-Cargo -Package 'vpr-live-proof' -Arguments @('candidate-bundle-extract',$bundle,$state,(Join-Path $ev 'provider-probe.json'),(Join-Path $ev 'conversation-attempt.json'))) -ne 0) { throw 'bundle extract failed' }
    if ((Get-FileHash $state -Algorithm SHA256).Hash -ne (Get-FileHash $labState -Algorithm SHA256).Hash) { throw 'PROVIDER_STATE_MISMATCH: refuse exit evidence' }
    if ((Invoke-Cargo -Package 'vpr-evaluation' -Bin 'vpr-rt0-supporting-scaffold' -Arguments @((Join-Path $ev 'supporting'),$state,$candidate)) -ne 0) { throw 'scaffold failed' }
    $owner = @(Collect-Sessions -Dir $ev -Since $since -Role 'owner' -MinVoice 1 -MinStop 0)
    Write-Host 'Next session MUST be conducted by another real person (visitor).'
    $visitor = @(Collect-Sessions -Dir $ev -Since (Get-Date) -Role 'visitor' -MinVoice 1 -MinStop 0)
    $ownerPath = Join-Path $ev 'session-owner.json'; $visitorPath = Join-Path $ev 'session-visitor.json'
    Move-Item $owner[-1].path $ownerPath -Force
    Move-Item $visitor[-1].path $visitorPath -Force
    $aggregate = Join-Path $ev 'bound-session-aggregate.json'
    if ((Invoke-Cargo -Package 'vpr-evaluation' -Bin 'vpr-rt0-session-aggregate' -Arguments @('bind',$state,$candidate,$ownerPath,$visitorPath) -StdoutFile $aggregate) -ne 0) { throw 'bind failed' }
    $readiness = Join-Path $EvidenceRoot 'runtime-readiness.json'
    $result = Invoke-Cargo -Package 'vpr-evaluation' -Bin 'vpr-rt0-runtime-readiness' -Arguments @($state,(Join-Path $ev 'conversation-attempt.json'),$aggregate,$candidate,$ownerPath,$visitorPath) -StdoutFile $readiness -AllowedExit @(0,1)
    $summary.stages['exit_readiness'] = @{exit_code=$result}
    Save-Summary
    if ($result -ne 0) { throw "Readiness failed, review $readiness" }
    if ((Invoke-Cargo -Package 'vpr-evaluation' -Bin 'vpr-rt0-runtime-supporting' -Arguments @((Join-Path $ev 'supporting'),$state,(Join-Path $ev 'conversation-attempt.json'),$aggregate,$candidate,$ownerPath,$visitorPath)) -ne 0) { throw 'runtime supporting failed' }
    $automation = Join-Path $PrivateInputDir 'rt0-reviewed-automation.json'
    if (-not (Test-Path $automation)) { throw "Reviewer must supply genuine CI/E2E identifiers in $automation" }
    if ((Invoke-Cargo -Package 'vpr-evaluation' -Bin 'vpr-rt0-automation-capture' -Arguments @($automation,(Join-Path $ev 'supporting'),$candidate)) -ne 0) { throw 'automation capture failed' }
    $observations = Join-Path $PrivateInputDir 'rt0-reviewed-observations.json'
    if ((Invoke-Cargo -Package 'vpr-evaluation' -Bin 'vpr-rt0-supporting-input-prefill' -Arguments @($observations,$state,$aggregate,$candidate,$ownerPath,$visitorPath)) -ne 0) { throw 'prefill failed' }
    Write-Host "STOP for real reviewer: fill $observations, known-limitations.md, private Golden inputs, cost and human acceptance."
    Write-Host 'Interim Piper Irina rights are NOT cleared for commercial release.'
    Write-Host 'After review run -Stage Finish.'
    exit 0
}
if ($Stage -eq 'Finish') {
    $state = Join-Path $ev 'provider-state.json'
    $aggregate = Join-Path $ev 'bound-session-aggregate.json'
    $ownerPath = Join-Path $ev 'session-owner.json'
    $visitorPath = Join-Path $ev 'session-visitor.json'
    $supporting = Join-Path $ev 'supporting'
    $observations = Join-Path $PrivateInputDir 'rt0-reviewed-observations.json'
    if ((Invoke-Cargo -Package 'vpr-evaluation' -Bin 'vpr-rt0-supporting-capture' -Arguments @($observations,$supporting,$state,$candidate)) -ne 0) { throw 'supporting capture failed' }
    if ((Invoke-Cargo -Package 'vpr-evaluation' -Bin 'vpr-rt0-supporting-preflight' -Arguments @($supporting,$state,$candidate)) -ne 0) { throw 'supporting preflight failed' }
    $golden = Join-Path $ev 'bound-golden-report.json'
    if ((Invoke-Cargo -Package 'vpr-evaluation' -Arguments @('docs/evaluation/rt0_golden_minimum.json',(Join-Path $ev 'private-golden-evidence.json'),(Join-Path $ev 'release-spec.md'),$state,$candidate) -StdoutFile $golden) -ne 0) { throw 'Golden failed' }
    if ((Invoke-Cargo -Package 'vpr-evaluation' -Bin 'vpr-rt0-owner-golden' -Arguments @((Join-Path $ev 'owner-golden-suite.json'),(Join-Path $ev 'owner-golden-evidence.json'),(Join-Path $ev 'release-spec.md'),$state,(Join-Path $ev 'owner-golden-report.json'),$candidate)) -ne 0) { throw 'owner Golden failed' }
    $exitManifest = Join-Path $ev 'exit-evidence.json'
    if ((Invoke-Cargo -Package 'vpr-evaluation' -Bin 'vpr-rt0-exit-assemble' -Arguments @($supporting,$golden,(Join-Path $ev 'owner-golden-suite.json'),(Join-Path $ev 'owner-golden-evidence.json'),(Join-Path $ev 'owner-golden-report.json'),$state,(Join-Path $ev 'provider-probe.json'),(Join-Path $ev 'conversation-attempt.json'),$aggregate,(Join-Path $ev 'release-spec.md'),$exitManifest,$candidate)) -ne 0) { throw 'exit assembly failed' }
    $inventory = Join-Path $EvidenceRoot 'inventory.json'
    $inventoryCode = Invoke-Cargo -Package 'vpr-evaluation' -Bin 'vpr-rt0-evidence-inventory' -Arguments @($ev,$candidate) -StdoutFile $inventory -AllowedExit @(0,1)
    $report = Join-Path $EvidenceRoot 'rt0-exit-report.json'
    $gate = Invoke-Cargo -Package 'vpr-evaluation' -Bin 'vpr-rt0-exit-evidence' -Arguments @($exitManifest,$golden,(Join-Path $ev 'private-golden-evidence.json'),(Join-Path $ev 'owner-golden-suite.json'),(Join-Path $ev 'owner-golden-evidence.json'),(Join-Path $ev 'owner-golden-report.json'),$state,(Join-Path $ev 'provider-probe.json'),(Join-Path $ev 'conversation-attempt.json'),$aggregate,$supporting,$ownerPath,$visitorPath,(Join-Path $ev 'release-spec.md'),$candidate) -StdoutFile $report -AllowedExit @(0,1,2)
    $summary.stages['finish'] = @{inventory_exit=$inventoryCode;exit_gate_exit=$gate}
    Save-Summary
    if ($gate -ne 0) { throw "RT0 exit gate failed: $gate (see $report)" }
    $result = Get-Content -LiteralPath $report -Raw -Encoding UTF8 | ConvertFrom-Json
    if (-not $result.ready) { throw 'RT0 exit evidence explicitly not ready' }
    Write-Host "RT0 exit PASS. Sanitized reports: $report, $inventory, $summaryPath"
    exit 0
}
) {
            throw "Yandex API secret was not received as a valid single-line ASCII token (length=$($key.Length)); paste the secret VALUE, not key ID, service account ID, asterisk, or masked preview. Never share the secret."
        }
        $uri = 'https://ai.api.cloud.yandex.net/v1/chat/completions'
        $model = "gpt://$folder/yandexgpt-5.1"
        $chosen = $null
        foreach ($auth in @('apikey','bearer')) {
            foreach ($projectHeader in @('openai-project','x-folder-id')) {
                $prefix = if ($auth -eq 'apikey') { 'Api-Key' } else { 'Bearer' }
                $hName = if ($projectHeader -eq 'openai-project') { 'OpenAI-Project' } else { 'x-folder-id' }
                $headers = @{ Authorization = "$prefix $key" }
                $headers[$hName] = $folder
                $r = Probe-Http -Uri $uri -Headers $headers -Body @{model=$model;messages=$messages;stream=$true;max_tokens=8}
                $r['auth'] = $auth; $r['project_header'] = $projectHeader
                $result.variants += $r
                if (-not $chosen -and $r.status -ge 200 -and $r.status -lt 300) {
                    $chosen = @{auth=$auth;project=$projectHeader;headers=$headers}
                }
            }
        }
        if (-not $chosen) {
            $probePath = Join-Path $Dir 'llm-wire-probe.json'
            ($result | ConvertTo-Json -Depth 7) | Set-Content -LiteralPath $probePath -Encoding UTF8
            $codes = @($result.variants | ForEach-Object { "$($_.auth)/$($_.project_header)=$($_.status)" }) -join ', '
            throw "Yandex wire probe rejected ($codes). Sanitized statuses saved to $probePath. Check API key scope, account permissions, billing, and model access."
        }
        $usage = Probe-Http -Uri $uri -Headers $chosen.headers -Body @{model=$model;messages=$messages;stream=$true;max_tokens=8;stream_options=@{include_usage=$true}}
        $usage['variant'] = 'stream_options.include_usage'
        $result.variants += $usage
        $usageFlag = if ($usage.status -ge 200 -and $usage.status -lt 300 -and $usage.usage_chunk) { 'on' } else { 'off' }
        $result['selected'] = @{auth_scheme=$chosen.auth;project_header=$chosen.project;stream_usage=$usageFlag}
        $result['model'] = $model
        $env:VPR_OWNER_LAB_LLM_PROJECT_ID = $folder
        $env:VPR_OWNER_LAB_LLM_AUTH_SCHEME = $chosen.auth
        $env:VPR_OWNER_LAB_LLM_PROJECT_HEADER = $chosen.project
        $env:VPR_OWNER_LAB_LLM_STREAM_USAGE = $usageFlag
    } else {
        $model = Read-Host 'ProxyAPI exact model ID from provider catalogue'
        $key = Read-SecretOnce 'ProxyAPI key for one-off wire probe'
        $uri = 'https://api.proxyapi.ru/v1/chat/completions'
        $headers = @{ Authorization = "Bearer $key" }
        $plain = Probe-Http -Uri $uri -Headers $headers -Body @{model=$model;messages=$messages;stream=$true;max_tokens=8}
        $usage = Probe-Http -Uri $uri -Headers $headers -Body @{model=$model;messages=$messages;stream=$true;max_tokens=8;stream_options=@{include_usage=$true}}
        $thinking = Probe-Http -Uri $uri -Headers $headers -Body @{model=$model;messages=$messages;stream=$true;max_tokens=8;thinking=@{type='disabled'};reasoning_effort='none'}
        $result.variants = @($plain,$usage,$thinking)
        if ($plain.status -lt 200 -or $plain.status -ge 300) { throw 'ProxyAPI plain streaming failed' }
        $usageFlag = if ($usage.status -ge 200 -and $usage.status -lt 300 -and $usage.usage_chunk) { 'on' } else { 'off' }
        $result['selected'] = @{auth_scheme='bearer';stream_usage=$usageFlag;thinking_fields_accepted=($thinking.status -ge 200 -and $thinking.status -lt 300)}
        $result['model'] = $model
        Remove-Item Env:VPR_OWNER_LAB_LLM_AUTH_SCHEME,Env:VPR_OWNER_LAB_LLM_PROJECT_HEADER,Env:VPR_OWNER_LAB_LLM_PROJECT_ID -ErrorAction SilentlyContinue
        $env:VPR_OWNER_LAB_LLM_STREAM_USAGE = $usageFlag
    }
    $key = $null
    [GC]::Collect()
    ($result | ConvertTo-Json -Depth 7) | Set-Content -LiteralPath (Join-Path $Dir 'llm-wire-probe.json') -Encoding UTF8
    return $result
}
function Launch-OwnerLab([string]$StateOut) {
    $env:VPR_OWNER_LAB_PROVIDER_STATE_OUT = $StateOut
    $launcher = Join-Path $repoRoot 'scripts\windows-owner-lab-restart.ps1'
    # The voice launcher deliberately supervises its subprocess until session exit.
    # Run it in a separate console so the acceptance driver can collect evidence.
    $arguments = '-NoProfile -ExecutionPolicy Bypass -File "' + $launcher + '" -Rt0Evidence -Piper -PiperVoice ' + $PiperVoice
    $process = Start-Process -FilePath 'powershell.exe' -ArgumentList $arguments -WorkingDirectory $repoRoot -PassThru
    $deadline = [DateTime]::UtcNow.AddMinutes(10)
    $ready = $false
    while ([DateTime]::UtcNow -lt $deadline) {
        if ($process.HasExited) {
            throw "Owner Lab launcher exited before readiness (exit code $($process.ExitCode)); inspect its console"
        }
        if (Test-Path -LiteralPath $StateOut -PathType Leaf) {
            try {
                $status = Invoke-RestMethod -Method Get -Uri 'http://127.0.0.1:8787/api/status' -TimeoutSec 2
                if ($status.session_state -in @('none','closed') -and
                    $status.conversation_readiness -eq 'text_and_voice' -and
                    $status.egress_enabled) {
                    $ready = $true
                    break
                }
            } catch {
                # Server and speech worker may still be starting.
            }
        }
        Start-Sleep -Milliseconds 500
    }
    if (-not $ready) { throw 'Owner Lab readiness or exact provider-state export did not complete' }
    Write-Host "Verified running Owner Lab and exact provider-state: $StateOut"
}
function Collect-Sessions {
    param([string]$Dir,[datetime]$Since,[string]$Role,[int]$MinVoice,[int]$MinStop)
    Write-Host "Please complete real $Role conversation in Owner Lab and download evidence snapshot."
    if ($MinVoice -gt 0) { Write-Host "At least $MinVoice completed Russian voice turns and $MinStop STOP observations required." }
    [void](Read-Host 'Press Enter after downloading snapshot into Downloads')
    $downloads = Join-Path $env:USERPROFILE 'Downloads'
    $found = @()
    foreach ($file in (Get-ChildItem -LiteralPath $downloads -Filter '*.json' -File | Where-Object { $_.LastWriteTime -ge $Since })) {
        try { $snap = Get-Content -LiteralPath $file.FullName -Raw -Encoding UTF8 | ConvertFrom-Json } catch { continue }
        if ($snap.schema_version -notlike 'rt0-owner-lab-session-evidence-*' -or $snap.candidate_sha -ne $candidate) { continue }
        $dest = Join-Path $Dir ("session-{0}-{1}.json" -f $Role,$snap.session_sequence)
        Copy-Item -LiteralPath $file.FullName -Destination $dest -Force
        $found += [pscustomobject]@{path=$dest; snapshot=$snap}
    }
    if ($found.Count -eq 0) { throw "No genuine $Role snapshot for $candidate" }
    $voiceCount = 0; $stopCount = 0
    foreach ($item in $found) {
        $voiceCount += @($item.snapshot.voice_attempts | Where-Object { $_.status -eq 'completed' }).Count
        $stopCount += @($item.snapshot.media_events | Where-Object { $_.kind -eq 'interruption_stopped' }).Count
    }
    if ($voiceCount -lt $MinVoice -or $stopCount -lt $MinStop) {
        throw "Insufficient real evidence: voice turns=$voiceCount/$MinVoice, STOP=$stopCount/$MinStop"
    }
    return $found
}
function State-Llm([string]$StatePath) {
    $state = Get-Content -LiteralPath $StatePath -Raw -Encoding UTF8 | ConvertFrom-Json
    $binding = @($state.providers | Where-Object { $_.role -eq 'llm' })
    if ($binding.Count -ne 1) { throw 'provider-state must bind one LLM' }
    return $binding[0]
}
function Set-Llm([string]$Llm) {
    if ($SkipSetLlm) { return }
    Write-Host "Configure $Llm. The provider is preselected; enter only the requested folder/model and hidden API key."
    if ((Invoke-Cargo -Package 'vpr-owner-lab' -Bin 'vpr-provider-credentials' -Arguments @('set-llm', $Llm)) -ne 0) { throw 'set-llm failed' }
}
if ($Stage -eq 'Compare') {
    $results = [ordered]@{}
    foreach ($llm in $Llms) {
        $dir = Join-Path $EvidenceRoot ('llm-compare\' + $llm)
        $null = New-Item -ItemType Directory -Force -Path $dir
        Set-Llm $llm
        $probe = Probe-Llm -Llm $llm -Dir $dir
        $statePath = Join-Path $dir 'provider-state.json'
        Remove-Item -LiteralPath $statePath -ErrorAction SilentlyContinue
        $since = Get-Date
        Launch-OwnerLab $statePath
        $binding = State-Llm $statePath
        if ($binding.provider -ne $llm) { throw "LLM mismatch: actual $($binding.provider), required $llm" }
        $sessions = Collect-Sessions -Dir $dir -Since $since -Role 'owner' -MinVoice $MinTurns -MinStop $MinStops
        $reportPath = Join-Path $dir 'latency-report.json'
        $args = @($llm,$binding.model_or_representation,$candidate,$statePath) + @($sessions | ForEach-Object { $_.path })
        $code = Invoke-Cargo -Package 'vpr-evaluation' -Bin 'vpr-rt0-llm-latency' -Arguments $args -StdoutFile $reportPath
        if ($code -ne 0) { throw "Latency evaluator failed for $llm" }
        $report = Get-Content -LiteralPath $reportPath -Raw -Encoding UTF8 | ConvertFrom-Json
        $results[$llm] = @{verdict=$report.verdict;model=$binding.model_or_representation;report=$reportPath}
    }
    $summary.stages['compare'] = $results
    Save-Summary
    Write-Host "Comparison completed; see $summaryPath"
    exit 0
}
if ($Stage -eq 'Exit') {
    Set-Llm $ExitLlm
    $inputs = @('probe.raw','reviewed-profile.json','owner.raw','visitor.raw') | ForEach-Object { Join-Path $PrivateInputDir $_ }
    foreach ($item in $inputs) { if (-not (Test-Path -LiteralPath $item)) { throw "Missing private input $item" } }
    if ((Invoke-Cargo -Package 'vpr-evaluation' -Bin 'vpr-rt0-evidence-prepare' -Arguments @($ev,$candidate)) -ne 0) { throw 'evidence-prepare failed' }
    $state = Join-Path $ev 'provider-state.json'
    $labState = Join-Path $ev 'owner-lab-provider-state.json'
    Remove-Item -LiteralPath $labState -ErrorAction SilentlyContinue
    $since = Get-Date
    Launch-OwnerLab $labState
    $env:VPR_OWNER_LAB_TTS_PROVIDER = 'piper'
    $env:VPR_OWNER_LAB_TTS_MODEL = 'piper'
    $env:VPR_OWNER_LAB_TTS_VOICE = $PiperVoice
    $env:VPR_OWNER_LAB_TTS_ENDPOINT = 'http://127.0.0.1:8897/v1/audio/speech'
    $env:VPR_OWNER_LAB_TTS_API_KEY = [Guid]::NewGuid().ToString('N')
    $env:VPR_OWNER_LAB_RT0_EVIDENCE = 'true'
    $profileName = if ($PiperVoice -eq 'ru_RU-denis-medium') { 'voice-profile.interim-male.json' } else { 'voice-profile.interim.json' }
    $env:VPR_OWNER_LAB_VOICE_PROFILE_PATH = Join-Path $env:LOCALAPPDATA ('Virtual-Persona-Runtime\voice-profiles\' + $profileName)
    if ((Invoke-Cargo -Package 'vpr-live-proof' -Arguments (@('doctor')+$inputs)) -ne 0) { throw 'doctor failed' }
    $bundle = Join-Path $ev 'candidate-bundle.json'
    $env:VPR_LIVE_PROOF_ALLOW_EGRESS = 'true'
    try {
        if ((Invoke-Cargo -Package 'vpr-live-proof' -Arguments (@('candidate-bundle')+$inputs+@($bundle))) -ne 0) { throw 'candidate-bundle failed' }
    } finally { Remove-Item Env:VPR_LIVE_PROOF_ALLOW_EGRESS -ErrorAction SilentlyContinue }
    if ((Invoke-Cargo -Package 'vpr-live-proof' -Arguments @('candidate-bundle-extract',$bundle,$state,(Join-Path $ev 'provider-probe.json'),(Join-Path $ev 'conversation-attempt.json'))) -ne 0) { throw 'bundle extract failed' }
    if ((Get-FileHash $state -Algorithm SHA256).Hash -ne (Get-FileHash $labState -Algorithm SHA256).Hash) { throw 'PROVIDER_STATE_MISMATCH: refuse exit evidence' }
    if ((Invoke-Cargo -Package 'vpr-evaluation' -Bin 'vpr-rt0-supporting-scaffold' -Arguments @((Join-Path $ev 'supporting'),$state,$candidate)) -ne 0) { throw 'scaffold failed' }
    $owner = @(Collect-Sessions -Dir $ev -Since $since -Role 'owner' -MinVoice 1 -MinStop 0)
    Write-Host 'Next session MUST be conducted by another real person (visitor).'
    $visitor = @(Collect-Sessions -Dir $ev -Since (Get-Date) -Role 'visitor' -MinVoice 1 -MinStop 0)
    $ownerPath = Join-Path $ev 'session-owner.json'; $visitorPath = Join-Path $ev 'session-visitor.json'
    Move-Item $owner[-1].path $ownerPath -Force
    Move-Item $visitor[-1].path $visitorPath -Force
    $aggregate = Join-Path $ev 'bound-session-aggregate.json'
    if ((Invoke-Cargo -Package 'vpr-evaluation' -Bin 'vpr-rt0-session-aggregate' -Arguments @('bind',$state,$candidate,$ownerPath,$visitorPath) -StdoutFile $aggregate) -ne 0) { throw 'bind failed' }
    $readiness = Join-Path $EvidenceRoot 'runtime-readiness.json'
    $result = Invoke-Cargo -Package 'vpr-evaluation' -Bin 'vpr-rt0-runtime-readiness' -Arguments @($state,(Join-Path $ev 'conversation-attempt.json'),$aggregate,$candidate,$ownerPath,$visitorPath) -StdoutFile $readiness -AllowedExit @(0,1)
    $summary.stages['exit_readiness'] = @{exit_code=$result}
    Save-Summary
    if ($result -ne 0) { throw "Readiness failed, review $readiness" }
    if ((Invoke-Cargo -Package 'vpr-evaluation' -Bin 'vpr-rt0-runtime-supporting' -Arguments @((Join-Path $ev 'supporting'),$state,(Join-Path $ev 'conversation-attempt.json'),$aggregate,$candidate,$ownerPath,$visitorPath)) -ne 0) { throw 'runtime supporting failed' }
    $automation = Join-Path $PrivateInputDir 'rt0-reviewed-automation.json'
    if (-not (Test-Path $automation)) { throw "Reviewer must supply genuine CI/E2E identifiers in $automation" }
    if ((Invoke-Cargo -Package 'vpr-evaluation' -Bin 'vpr-rt0-automation-capture' -Arguments @($automation,(Join-Path $ev 'supporting'),$candidate)) -ne 0) { throw 'automation capture failed' }
    $observations = Join-Path $PrivateInputDir 'rt0-reviewed-observations.json'
    if ((Invoke-Cargo -Package 'vpr-evaluation' -Bin 'vpr-rt0-supporting-input-prefill' -Arguments @($observations,$state,$aggregate,$candidate,$ownerPath,$visitorPath)) -ne 0) { throw 'prefill failed' }
    Write-Host "STOP for real reviewer: fill $observations, known-limitations.md, private Golden inputs, cost and human acceptance."
    Write-Host 'Interim Piper Irina rights are NOT cleared for commercial release.'
    Write-Host 'After review run -Stage Finish.'
    exit 0
}
if ($Stage -eq 'Finish') {
    $state = Join-Path $ev 'provider-state.json'
    $aggregate = Join-Path $ev 'bound-session-aggregate.json'
    $ownerPath = Join-Path $ev 'session-owner.json'
    $visitorPath = Join-Path $ev 'session-visitor.json'
    $supporting = Join-Path $ev 'supporting'
    $observations = Join-Path $PrivateInputDir 'rt0-reviewed-observations.json'
    if ((Invoke-Cargo -Package 'vpr-evaluation' -Bin 'vpr-rt0-supporting-capture' -Arguments @($observations,$supporting,$state,$candidate)) -ne 0) { throw 'supporting capture failed' }
    if ((Invoke-Cargo -Package 'vpr-evaluation' -Bin 'vpr-rt0-supporting-preflight' -Arguments @($supporting,$state,$candidate)) -ne 0) { throw 'supporting preflight failed' }
    $golden = Join-Path $ev 'bound-golden-report.json'
    if ((Invoke-Cargo -Package 'vpr-evaluation' -Arguments @('docs/evaluation/rt0_golden_minimum.json',(Join-Path $ev 'private-golden-evidence.json'),(Join-Path $ev 'release-spec.md'),$state,$candidate) -StdoutFile $golden) -ne 0) { throw 'Golden failed' }
    if ((Invoke-Cargo -Package 'vpr-evaluation' -Bin 'vpr-rt0-owner-golden' -Arguments @((Join-Path $ev 'owner-golden-suite.json'),(Join-Path $ev 'owner-golden-evidence.json'),(Join-Path $ev 'release-spec.md'),$state,(Join-Path $ev 'owner-golden-report.json'),$candidate)) -ne 0) { throw 'owner Golden failed' }
    $exitManifest = Join-Path $ev 'exit-evidence.json'
    if ((Invoke-Cargo -Package 'vpr-evaluation' -Bin 'vpr-rt0-exit-assemble' -Arguments @($supporting,$golden,(Join-Path $ev 'owner-golden-suite.json'),(Join-Path $ev 'owner-golden-evidence.json'),(Join-Path $ev 'owner-golden-report.json'),$state,(Join-Path $ev 'provider-probe.json'),(Join-Path $ev 'conversation-attempt.json'),$aggregate,(Join-Path $ev 'release-spec.md'),$exitManifest,$candidate)) -ne 0) { throw 'exit assembly failed' }
    $inventory = Join-Path $EvidenceRoot 'inventory.json'
    $inventoryCode = Invoke-Cargo -Package 'vpr-evaluation' -Bin 'vpr-rt0-evidence-inventory' -Arguments @($ev,$candidate) -StdoutFile $inventory -AllowedExit @(0,1)
    $report = Join-Path $EvidenceRoot 'rt0-exit-report.json'
    $gate = Invoke-Cargo -Package 'vpr-evaluation' -Bin 'vpr-rt0-exit-evidence' -Arguments @($exitManifest,$golden,(Join-Path $ev 'private-golden-evidence.json'),(Join-Path $ev 'owner-golden-suite.json'),(Join-Path $ev 'owner-golden-evidence.json'),(Join-Path $ev 'owner-golden-report.json'),$state,(Join-Path $ev 'provider-probe.json'),(Join-Path $ev 'conversation-attempt.json'),$aggregate,$supporting,$ownerPath,$visitorPath,(Join-Path $ev 'release-spec.md'),$candidate) -StdoutFile $report -AllowedExit @(0,1,2)
    $summary.stages['finish'] = @{inventory_exit=$inventoryCode;exit_gate_exit=$gate}
    Save-Summary
    if ($gate -ne 0) { throw "RT0 exit gate failed: $gate (see $report)" }
    $result = Get-Content -LiteralPath $report -Raw -Encoding UTF8 | ConvertFrom-Json
    if (-not $result.ready) { throw 'RT0 exit evidence explicitly not ready' }
    Write-Host "RT0 exit PASS. Sanitized reports: $report, $inventory, $summaryPath"
    exit 0
}
