@echo off
setlocal
rem Use the installed Russian female Windows voice. Piper remains optional.
call "%~dp0scripts\start-rt0-sapi.cmd"
exit /b %ERRORLEVEL%
