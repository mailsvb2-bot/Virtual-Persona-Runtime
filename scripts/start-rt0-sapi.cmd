@echo off
setlocal
rem Native female Russian SAPI, user-confirmed, protected Echo and exact RT0 SHA.
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0windows-owner-lab-restart.ps1" -SystemVoice -Rt0Evidence
if errorlevel 1 (
  echo Windows voice or RT0 Owner Lab failed to start. No RT0 exit was claimed.
  pause
  exit /b 1
)
endlocal
