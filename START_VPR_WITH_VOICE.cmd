@echo off
setlocal EnableExtensions DisableDelayedExpansion
rem Single entrypoint for Windows RT0 avatar + Russian speech.
rem Prefer explicit interim neural Piper over rejected robotic Windows SAPI.
rem Do NOT silently fall back if the selected voice or real-provider preflight fails.
set "VPR_LAUNCHER=%~dp0scripts\windows-owner-lab-restart.ps1"
if not exist "%VPR_LAUNCHER%" (
  echo ERROR: VPR launcher missing. Use a complete Git checkout.
  exit /b 1
)
if /I "%~1"=="--system-voice" (
  if not "%~2"=="" goto usage
  powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%VPR_LAUNCHER%" -SystemVoice
  goto result
)
if /I "%~1"=="--piper-denis" (
  if not "%~2"=="" goto usage
  powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%VPR_LAUNCHER%" -Piper -PiperVoice ru_RU-denis-medium
  goto result
)
if not "%~1"=="" goto usage
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%VPR_LAUNCHER%" -Piper
:result
set "VPR_EXIT=%ERRORLEVEL%"
if not "%VPR_EXIT%"=="0" (
  echo ERROR: RT0 was not launched successfully. No audio/video success is claimed.
  exit /b %VPR_EXIT%
)
exit /b 0
:usage
echo Usage: START_VPR_WITH_VOICE.cmd [--piper-denis ^| --system-voice]
exit /b 2
