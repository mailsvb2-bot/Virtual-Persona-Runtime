param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('chatterbox-v3', 'qwen-base', 'qwen-design')]
    [string]$Engine
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# A voice audition never uses the live Owner Lab environment or its secrets.
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$test = Join-Path $repo 'tests/e2e/test_rt0_multilingual_voice_audition.py'
$generator = Join-Path $repo 'scripts/rt0_multilingual_voice_audition.py'
$root = Join-Path $(if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { $env:TEMP }) ('vpr-voice-audition-' + $Engine)
$venv = Join-Path $root 'venv'
$python = Join-Path $venv 'Scripts/python.exe'
$output = Join-Path $repo 'audio-candidates'

Set-Location -LiteralPath $repo
& py -3.11 $test
if ($LASTEXITCODE -ne 0) { throw 'Isolated voice audition contract failed' }
if (-not (Test-Path -LiteralPath $python)) {
    & py -3.11 -m venv $venv
    if ($LASTEXITCODE -ne 0) { throw 'Isolated Python 3.11 environment unavailable' }
}
& $python -m pip install --disable-pip-version-check --upgrade pip
if ($LASTEXITCODE -ne 0) { throw 'pip setup failed' }
if ($Engine -eq 'chatterbox-v3') {
    & $python -m pip install --disable-pip-version-check chatterbox-tts==0.1.7
} else {
    & $python -m pip install --disable-pip-version-check qwen-tts==0.1.1
}
if ($LASTEXITCODE -ne 0) { throw 'Pinned upstream neural TTS install failed' }

if ($Engine -eq 'qwen-design') {
    & $python $generator --engine $Engine --output-dir $output
} else {
    if (-not $env:GH_TOKEN) { throw 'A pinned synthetic reference requires GitHub Actions read scope' }
    $referenceFolder = Join-Path $root 'reference'
    New-Item -ItemType Directory -Force -Path $referenceFolder | Out-Null
    & gh run download 38226686115 --repo mailsvb2-bot/Virtual-Persona-Runtime -n silero-female-cis-ru-sha-60e29a259e6de6416ee67d8139839c9caf0d882e -D $referenceFolder
    if ($LASTEXITCODE -ne 0) { throw 'Known synthetic MIT-voice fixture unavailable' }
    $reference = Join-Path $referenceFolder 'ru_ekaterina.wav'
    & $python -c 'import sys; sys.path.insert(0, "scripts"); from pathlib import Path; from rt0_multilingual_voice_audition import validate_reference; validate_reference(Path(sys.argv[1]))' $reference
    if ($LASTEXITCODE -ne 0) { throw 'Synthetic reference WAV integrity failed' }
    & $python $generator --engine $Engine --reference $reference --output-dir $output
}
if ($LASTEXITCODE -ne 0) { throw 'Real neural Russian voice inference failed' }
Write-Host ('NEURAL_AUDITION_COMPLETED: ' + $Engine)
