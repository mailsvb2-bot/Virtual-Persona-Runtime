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

foreach ($name in @('Assert-ExpectedListener', 'Stop-PortListener')) {
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
function Get-Process {
    param($Id, $ErrorAction)
    [pscustomobject]@{ Path = $script:runningExe }
}
function Stop-Process {
    param($Id, [switch]$Force, $ErrorAction)
    $script:stopped += $Id
}
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
Write-Host 'Windows launcher listener ownership regression: PASS'
