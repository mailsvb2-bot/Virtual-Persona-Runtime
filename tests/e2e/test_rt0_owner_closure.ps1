# Offline safety contract for the guided RT0 owner closure script.
$ErrorActionPreference = 'Stop'
$sourcePath = Join-Path $PSScriptRoot '..\..\scripts\rt0-owner-closure.ps1'
$parseErrors = $null
$tokens = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile(
    (Resolve-Path $sourcePath).Path, [ref]$tokens, [ref]$parseErrors
)
if ($parseErrors.Count -gt 0) { throw 'RT0 owner closure PowerShell syntax invalid' }
$source = Get-Content -LiteralPath $sourcePath -Raw
foreach ($required in @(
    "[ValidateSet('Compare','Exit','Finish')]",
    "RT0 evidence requires a clean Git worktree",
    "RT0 evidence must be outside the Git worktree",
    'Read-Host -AsSecureString',
    'VPR_OWNER_LAB_PROVIDER_STATE_OUT',
    'VPR_OWNER_LAB_LLM_AUTH_SCHEME',
    'VPR_OWNER_LAB_LLM_PROJECT_HEADER',
    'VPR_OWNER_LAB_LLM_STREAM_USAGE',
    'vpr-rt0-llm-latency',
    'vpr-rt0-runtime-readiness',
    'vpr-rt0-supporting-preflight',
    'vpr-rt0-exit-evidence',
    'provider-state.json',
    'session-owner.json',
    'session-visitor.json',
    'PiperVoice',
    # The private voice launcher blocks while supervising its child. The
    # acceptance console must remain free to collect live session evidence.
    "Start-Process -FilePath 'powershell.exe'",
    'Verified running Owner Lab and exact provider-state',
    'Get-FileHash',
    'voice turns',
    'STOP'
)) {
    if (-not $source.Contains($required)) { throw "RT0 owner closure safety contract missing: $required" }
}
if ($source.Contains('Invoke-Expression') -or $source.Contains('ConvertFrom-SecureString -AsPlainText')) {
    throw 'RT0 owner closure must never evaluate arbitrary commands or persist plaintext credentials'
}
Write-Host 'RT0 owner closure safety contract: PASS'
