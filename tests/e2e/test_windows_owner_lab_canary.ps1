# Independent behavioral checks for real launcher canary functions.
$ErrorActionPreference = 'Stop'
$script:Port = 8787
$launcher = Join-Path $PSScriptRoot '..\..\scripts\windows-owner-lab-restart.ps1'
$astTokens = $null
$astErrors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile(
    (Resolve-Path $launcher).Path, [ref]$astTokens, [ref]$astErrors
)
if ($astErrors.Count -gt 0) { throw 'Launcher PowerShell syntax invalid' }
foreach ($name in @(
    'Get-AvailableCanaryPort', 'Assert-ExpectedListener',
    'Assert-CanaryReadiness', 'Wait-CanaryLabReady', 'Stop-CanaryLab'
)) {
    $functions = @($ast.FindAll({
        param($node)
        $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq $name
    }, $true))
    if ($functions.Count -ne 1) { throw "Missing canonical launcher function $name" }
    . ([ScriptBlock]::Create($functions[0].Extent.Text))
}
function Assert-Rejected([scriptblock]$block, [string]$expected) {
    $reason = $null
    try { & $block } catch { $reason = $_.Exception.Message }
    if (-not $reason -or $reason -notmatch $expected) {
        throw "Expected rejection ($expected), obtained: $reason"
    }
}
$ready = [pscustomobject]@{
    egress_enabled = $true
    conversation_readiness = 'text_and_voice'
    session_state = 'none'
}
$boot = [pscustomobject]@{ egress_enabled = $true; rt0_evidence_mode = $true }
Assert-CanaryReadiness -Bootstrap $boot -Status $ready -StrictEvidence $true
Assert-Rejected { Assert-CanaryReadiness -Bootstrap ([pscustomobject]@{egress_enabled = $false;rt0_evidence_mode = $true}) -Status $ready -StrictEvidence $true } 'egress'
Assert-Rejected { Assert-CanaryReadiness -Bootstrap ([pscustomobject]@{egress_enabled = $true;rt0_evidence_mode = $false}) -Status $ready -StrictEvidence $true } 'RT0'
Assert-Rejected { Assert-CanaryReadiness -Bootstrap $boot -Status ([pscustomobject]@{egress_enabled = $true;conversation_readiness = 'none';session_state = 'none'}) -StrictEvidence $true } 'text-and-voice'
Assert-Rejected { Assert-CanaryReadiness -Bootstrap $boot -Status ([pscustomobject]@{egress_enabled = $true;conversation_readiness = 'text_and_voice';session_state = 'active'}) -StrictEvidence $true } 'active session'

$script:busyCanaryPorts = @(8788, 8789)
function Get-NetTCPConnection {
    param($LocalPort, $State, $ErrorAction)
    if ($LocalPort -in $script:busyCanaryPorts) {
        [pscustomobject]@{ OwningProcess = 51881 }
    }
}
if ((Get-AvailableCanaryPort -PrimaryPort 8787) -ne 8790) { throw 'Canary should skip occupied ports' }
$script:busyCanaryPorts = @(8788..8819)
Assert-Rejected { Get-AvailableCanaryPort -PrimaryPort 8787 } 'No free canary port'
Assert-Rejected { Get-AvailableCanaryPort -PrimaryPort 65535 } 'cannot support'

$script:probeCount = 0
$script:uiAssetCount = 0
$script:trustedExe = 'C:\repo\target\rt0-live\b\vpr-owner-lab.exe'
$script:observedExe = 'C:\Windows\System32\foreign.exe'
$script:observedPid = 51881
$script:busyCanaryPorts = @(8788)
$script:closedCount = 0
function Get-Process {
    param($Id, $ErrorAction)
    $proc = [pscustomobject]@{
        Id = $Id
        Path = $script:observedExe
        Handle = 12345
    }
    $proc | Add-Member -MemberType ScriptMethod -Name Close -Value { $script:closedCount += 1 }
    return $proc
}
function Invoke-RestMethod {
    param($Method, $Uri, $TimeoutSec)
    $script:probeCount++
    if ($Uri.EndsWith('/api/bootstrap')) { return $boot }
    if ($Uri.EndsWith('/api/status')) { return $ready }
    throw 'Unexpected private canary request'
}
function Invoke-WebRequest {
    param($UseBasicParsing, $Method, $Uri, $TimeoutSec)
    $script:uiAssetCount++
    if ($Uri -notmatch '^http://127[.]0[.]0[.]1:8788(/|/app[.]js)$') {
        throw 'Unexpected canary browser URL'
    }
    return [pscustomobject]@{ StatusCode = 200; Content = 'nonempty verified browser asset' }
}
$process = [pscustomobject]@{ Id = 51881; HasExited = $false }
Assert-Rejected {
    Wait-CanaryLabReady -Process $process -ListenPort 8788 -ExpectedExe $script:trustedExe -TimeoutSeconds 2
} 'unexpected process'
if ($script:probeCount -ne 0) { throw 'HTTP probe reached an unverified process' }
$script:observedExe = $script:trustedExe
Wait-CanaryLabReady -Process $process -ListenPort 8788 -ExpectedExe $script:trustedExe -TimeoutSeconds 2
if ($script:probeCount -ne 2 -or $script:uiAssetCount -ne 2) {
    throw 'Canary must check bootstrap, status and embedded browser assets'
}
$process.HasExited = $true
Assert-Rejected {
    Wait-CanaryLabReady -Process $process -ListenPort 8788 -ExpectedExe $script:trustedExe -TimeoutSeconds 2
} 'exited'
$process.HasExited = $false
$script:observedPid = 51999
function Get-NetTCPConnection {
    param($LocalPort, $State, $ErrorAction)
    if ($LocalPort -eq 8788) { [pscustomobject]@{ OwningProcess = $script:observedPid } }
}
Assert-Rejected {
    Wait-CanaryLabReady -Process $process -ListenPort 8788 -ExpectedExe $script:trustedExe -TimeoutSeconds 2
} 'different process'
if ($script:probeCount -ne 2) { throw 'Wrong canary PID must never receive an HTTP probe' }

# Cleanup must kill ONLY the retained Start-Process object and verify that the
# port is no longer listening. No PID-only Stop-Process is permitted.
$script:canaryKillCount = 0
$script:canaryCloseCount = 0
$script:canaryExited = $false
$staged = [pscustomobject]@{ HasExited = $false; Id = 51881 }
$staged | Add-Member -MemberType ScriptMethod -Name Kill -Value {
    $script:canaryKillCount++
    $this.HasExited = $true
    $script:canaryExited = $true
}
$staged | Add-Member -MemberType ScriptMethod -Name WaitForExit -Value {
    param($milliseconds)
    return $this.HasExited
}
$staged | Add-Member -MemberType ScriptMethod -Name Close -Value {
    $script:canaryCloseCount++
}
# Once our process exits, Windows releases the listener.
$script:observedPid = 0
function Get-NetTCPConnection {
    param($LocalPort, $State, $ErrorAction)
    if ($LocalPort -eq 8788 -and $script:observedPid -gt 0) {
        [pscustomobject]@{ OwningProcess = $script:observedPid }
    }
}
Stop-CanaryLab -Process $staged -ListenPort 8788
if ($script:canaryKillCount -ne 1 -or $script:canaryCloseCount -ne 1) {
    throw 'Retained canary process handle must be stopped and closed once'
}
Write-Host 'RT0 candidate canary startup, privacy, readiness and cleanup regression: PASS'
