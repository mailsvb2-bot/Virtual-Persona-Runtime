@echo off
setlocal
rem Explicit RT0 test-mode launch: frozen Git candidate, Firefox, automatic Piper.
rem For unmerged PR testing, do not switch to main or bypass exact-SHA evidence.
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0windows-owner-lab-restart.ps1" -Piper -Rt0Evidence
if errorlevel 1 (
    echo VPR/Piper did not start. No success has been recorded.
    pause
    exit /b 1
)
endlocal
