# Execute the real launcher safety functions in isolation, with stubbed OS commands.
# A foreign listener must NEVER be terminated or cause other side effects.
$ErrorActionPreference = 'Stop'
$script:Port = 8787
$script:expectedExe = 'C:\vpr\target\debug\vpr-owner-lab.exe'
$launcher = Join-Path $PSScriptRoot '..\..\scripts\windows-owner-lab-restart.ps1'
$parseErrors = $null
$tokens = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile(
    (Resolve-Path $launcher).Path, [ref]$tokens, [ref]$parseErrors
)
if ($parseErrors.Count -gt 0) { throw "Windows launcher has PowerShell parse errors" }

foreach ($name in @('Assert-ExpectedListener', 'Stop-PortListener', 'Get-Rt0GitCandidate')) {
    $matching = @($ast.FindAll({
        param($node)
        $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
        $node.Name -eq $name
    }, $true))
    if ($matching.Count -ne 1) { throw "Expected exactly one $name function" }
    . ([ScriptBlock]::Create($matching[0].Extent.Text))
}

$script:listenerPresent = $true
$script:runningExe = 'C:\Windows\System32\other-service.exe'
$script:stopped = @()
$script:waited = $false

function Get-NetTCPConnection {
    param($LocalPort, $State, $ErrorAction)
    if ($script:listenerPresent) { [pscustomobject]@{ OwningProcess = 51773 } }
}
$script:processLookups = 0
$script:closed = 0
function Get-Process {
    param($Id, $ErrorAction)
    $script:processLookups += 1
    $process = [pscustomobject]@{
        Id = $Id
        Path = $script:runningExe
        Handle = 12345
    }
    $process | Add-Member -MemberType ScriptMethod -Name Kill -Value {
        $script:stopped += $this.Id
        # After the verified process exits, Windows could recycle its numeric
        # PID. The production shutdown must not query that PID again.
        $script:runningExe = 'C:\Windows\System32\replacement.exe'
    }
    $process | Add-Member -MemberType ScriptMethod -Name Close -Value {
        $script:closed += 1
    }
    return $process
}
function Stop-Process { throw 'Unsafe PID-only Stop-Process must never be called' }
function Wait-LabDown { $script:waited = $true }

$rejected = $false
try { Stop-PortListener -ExpectedExe $script:expectedExe } catch {
    $rejected = $_.Exception.Message -match 'unexpected process'
}
if (-not $rejected -or $script:stopped.Count -ne 0 -or $script:waited) {
    throw 'Foreign port listener was not rejected before termination'
}

$script:runningExe = ''
$rejected = $false
try { Stop-PortListener -ExpectedExe $script:expectedExe } catch {
    $rejected = $_.Exception.Message -match 'Could not resolve executable'
}
if (-not $rejected -or $script:stopped.Count -ne 0) {
    throw 'Uninspectable port listener was not rejected before termination'
}

$script:listenerPresent = $false
Stop-PortListener -ExpectedExe $script:expectedExe
if ($script:stopped.Count -ne 0 -or $script:waited) {
    throw 'Empty port unexpectedly triggered termination'
}

$script:listenerPresent = $true
$script:runningExe = $script:expectedExe
Stop-PortListener -ExpectedExe $script:expectedExe
if ($script:stopped.Count -ne 1 -or $script:stopped[0] -ne 51773 -or -not $script:waited) {
    throw 'Verified stale Owner Lab listener was not stopped exactly once'
}
if ($script:runningExe -ne 'C:\Windows\System32\replacement.exe' -or $script:processLookups -ne 3) {
    throw 'Shutdown re-resolved a numeric PID instead of keeping the verified Process handle'
}
if ($script:closed -ne 3) {
    throw 'Verified and rejected process handles were not consistently closed'
}
Write-Host 'Windows launcher listener ownership regression: PASS'
# A downloaded GitHub ZIP is not an exact-candidate checkout. Fail before
# trying git or touching any live listener.
$zipRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("vpr-source-zip-" + [guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $zipRoot
try {
    $rejectedZip = $false
    try {
        $null = Get-Rt0GitCandidate -RepositoryRoot $zipRoot
    } catch {
        $rejectedZip = $_.Exception.Message -match 'Git clone, not a GitHub ZIP'
    }
    if (-not $rejectedZip) { throw 'GitHub ZIP checkout must fail with a useful Git clone instruction' }
} finally {
    Remove-Item -LiteralPath $zipRoot -Force -Recurse
}
Write-Host 'RT0 ZIP checkout diagnostic regression: PASS'

