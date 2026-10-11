@echo off
setlocal
rem Double-click from an exact Git checkout to prepare a Russian voice and open VPR.
call "%~dp0scripts\start-rt0-piper.cmd"
exit /b %ERRORLEVEL%
