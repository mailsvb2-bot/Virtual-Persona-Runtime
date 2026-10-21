# A REAL Windows process E2E using only local dummy provider endpoints.
# Unlike the AST mocks, this starts the compiled Rust Owner Lab twice,
# verifies both instances, preserves the old listener during candidate failure,
# and cuts over without any external provider call. NOT RT0 playback evidence.
$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$exe = Join-Path $repoRoot 'target\debug\vpr-owner-lab.exe'
if (-not (Test-Path -LiteralPath $exe)) { throw 'Compiled Rust Owner Lab binary is required' }
$script:Port = 18780
$privatePort = 18781
$script:expectedExe = $exe
$launcher = Join-Path $repoRoot 'scripts\windows-owner-lab-restart.ps1'
$tokens = $null
$errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile(
    (Resolve-Path $launcher).Path, [ref]$tokens, [ref]$errors
)
if ($errors.Count -gt 0) { throw 'Windows launcher syntax invalid' }
foreach ($name in @(
    'Get-AvailableCanaryPort', 'Assert-ExpectedListener',
    'Assert-CanaryReadiness', 'Wait-CanaryLabReady', 'Stop-CanaryLab'
)) {
    $f = @($ast.FindAll({
        param($node)
        $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq $name
    }, $true))
    if ($f.Count -ne 1) { throw "Missing live launcher function $name" }
    . ([ScriptBlock]::Create($f[0].Extent.Text))
}
# A synthetic but structurally reviewed Persona is required for the canonical
# text_and_voice readiness status. This is a hermetic CI fixture only, not a
# human attestation or persisted user data.
$fixturePath = Join-Path $env:RUNNER_TEMP 'vpr-canary-synthetic-persona.json'
$stagedFixturePath = Join-Path $env:RUNNER_TEMP 'vpr-canary-staged-persona.json'
$syntheticPersona = @{
    schema_version = 'vpr-reviewed-owner-persona-2'
    snapshot = @{
        persona_id = 'ci-only-owner'
        persona_version = 3
        claims = @(
            @{
                claim_id = 'ci-only-style'
                history_complete = $true
                revisions = @(
                    @{
                        revision = 1
                        statement = 'CI fixture, not a real human statement'
                        kind = 'preference'
                        source = 'owner'
                        verification = 'unverified'
                        derivation = 'direct'
                    },
                    @{
                        revision = 2
                        statement = 'CI fixture, not a real human statement'
                        kind = 'preference'
                        source = 'owner'
                        verification = 'owner_verified'
                        derivation = 'direct'
                    }
                )
            }
        )
    }
}
[IO.File]::WriteAllText(
    $fixturePath,
    ($syntheticPersona | ConvertTo-Json -Depth 20),
    [Text.UTF8Encoding]::new($false)
)
$envOverrides = @{
    VPR_PROVIDER_CREDENTIAL_SOURCE = 'environment'
    VPR_OWNER_LAB_AVATAR_PROVIDER = 'did'
    VPR_DID_ENDPOINT = 'http://127.0.0.1:18790'
    VPR_DID_API_KEY = 'windows-canary-fixture-only'
    VPR_DID_AGENT_ID = 'windows-canary-did-agent'
    VPR_DID_ECHO_ENABLED = 'false'
    VPR_OWNER_LAB_ALLOW_EGRESS = 'true'
    VPR_OWNER_LAB_RT0_EVIDENCE = 'false'
    VPR_OWNER_LAB_STT_PROVIDER = 'deepgram'
    VPR_OWNER_LAB_STT_ENDPOINT = 'http://127.0.0.1:18790/v1/listen'
    VPR_OWNER_LAB_STT_API_KEY = 'windows-canary-stt-fixture'
    VPR_OWNER_LAB_STT_MODEL = 'nova-3'
    VPR_OWNER_LAB_LLM_PROVIDER = 'deepseek'
    VPR_OWNER_LAB_LLM_ENDPOINT = 'http://127.0.0.1:18790/v1/chat/completions'
    VPR_OWNER_LAB_LLM_API_KEY = 'windows-canary-llm-fixture'
    VPR_OWNER_LAB_LLM_MODEL = 'deepseek-flash'
    VPR_OWNER_LAB_PERSONA_STORE_PATH = $fixturePath
}
$original = @{}
foreach ($name in ($envOverrides.Keys + @('VPR_OWNER_LAB_PORT'))) {
    $original[$name] = [Environment]::GetEnvironmentVariable($name,'Process')
}
foreach ($key in $envOverrides.Keys) {
    [Environment]::SetEnvironmentVariable($key, $envOverrides[$key], 'Process')
}
Copy-Item -LiteralPath $fixturePath -Destination $stagedFixturePath -Force
$old = $null
$staged = $null
$bad = $null
$final = $null
try {
    if (@(Get-NetTCPConnection -LocalPort $script:Port -State Listen -ErrorAction SilentlyContinue).Count -ne 0) {
        throw 'Old Owner Lab E2E port unexpectedly occupied'
    }
    $env:VPR_OWNER_LAB_PORT = [string]$script:Port
    $old = Start-Process -FilePath $exe -ArgumentList '--allow-egress' -WorkingDirectory $repoRoot -PassThru
    Wait-CanaryLabReady -Process $old -ListenPort $script:Port -ExpectedExe $exe -StrictEvidence $false -TimeoutSeconds 25

    $env:VPR_OWNER_LAB_PORT = [string]$privatePort
    $env:VPR_OWNER_LAB_PERSONA_STORE_PATH = $stagedFixturePath
    $staged = Start-Process -FilePath $exe -ArgumentList '--allow-egress' -WorkingDirectory $repoRoot -PassThru
    Wait-CanaryLabReady -Process $staged -ListenPort $privatePort -ExpectedExe $exe -StrictEvidence $false -TimeoutSeconds 25
    $oldStatus = Invoke-RestMethod -Method Get -Uri "http://127.0.0.1:$script:Port/api/status" -TimeoutSec 2
    if (-not $oldStatus.egress_enabled -or $old.HasExited) {
        throw 'Canary launch interrupted a working Owner Lab'
    }
    Stop-CanaryLab -Process $staged -ListenPort $privatePort
    $staged = $null

    # A broken provider selection must fail on the new port WITHOUT touching
    # the existing listener or silently falling back to real user credentials.
    $env:VPR_OWNER_LAB_LLM_PROVIDER = 'intentionally-unsupported-canary-provider'
    $bad = Start-Process -FilePath $exe -ArgumentList '--allow-egress' -WorkingDirectory $repoRoot -PassThru
    $failure = $null
    try {
        Wait-CanaryLabReady -Process $bad -ListenPort $privatePort -ExpectedExe $exe -StrictEvidence $false -TimeoutSeconds 8
    } catch { $failure = $_.Exception.Message }
    if (-not $failure) { throw 'Invalid provider unexpectedly passed full Owner Lab startup' }
    Stop-CanaryLab -Process $bad -ListenPort $privatePort
    $bad = $null
    $env:VPR_OWNER_LAB_LLM_PROVIDER = 'deepseek'
    $env:VPR_OWNER_LAB_PERSONA_STORE_PATH = $fixturePath
    $oldStatus = Invoke-RestMethod -Method Get -Uri "http://127.0.0.1:$script:Port/api/status" -TimeoutSec 2
    if (-not $oldStatus.egress_enabled -or $old.HasExited) {
        throw 'Failed canary terminated the previous Owner Lab'
    }

    # Normal clean handoff between two real instances after all checks pass.
    Stop-CanaryLab -Process $old -ListenPort $script:Port
    $old = $null
    $env:VPR_OWNER_LAB_PORT = [string]$script:Port
    $final = Start-Process -FilePath $exe -ArgumentList '--allow-egress' -WorkingDirectory $repoRoot -PassThru
    Wait-CanaryLabReady -Process $final -ListenPort $script:Port -ExpectedExe $exe -StrictEvidence $false -TimeoutSeconds 25
    Write-Host 'Real Windows two-instance Owner Lab canary and failure-preserving handoff: PASS (mock credentials; no external calls)'
} finally {
    foreach ($procWithPort in @(
        @{ process = $final; port = $script:Port },
        @{ process = $bad; port = $privatePort },
        @{ process = $staged; port = $privatePort },
        @{ process = $old; port = $script:Port }
    )) {
        if ($null -ne $procWithPort.process) {
            try { Stop-CanaryLab -Process $procWithPort.process -ListenPort $procWithPort.port }
            catch { Write-Warning "Owner Lab cleanup failed; CI must report this error: $($_.Exception.Message)"; throw }
        }
    }
    foreach ($key in $original.Keys) {
        [Environment]::SetEnvironmentVariable($key, $original[$key], 'Process')
    }
    Remove-Item -LiteralPath $fixturePath -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $stagedFixturePath -Force -ErrorAction SilentlyContinue
}
