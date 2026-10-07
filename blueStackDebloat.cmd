@echo off
setlocal
set "APP=%~dp0BluestacksDebloat.exe"
if not exist "%APP%" set "APP=%~dp0dist\BluestacksDebloat.exe"
if not exist "%APP%" (
  echo BluestacksDebloat.exe was not found. Run tools\build.ps1 or use a compiled release.
  pause
  exit /b 1
)
"%APP%" %*