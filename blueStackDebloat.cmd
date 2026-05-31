@echo off
setlocal
title Bluestacks-Debloat

rem --- self-elevate to Administrator -------------------------------------------------
net session >nul 2>&1
if %errorlevel% neq 0 (
    echo Requesting administrator privileges...
    powershell -NoProfile -Command "Start-Process -Verb RunAs -FilePath '%~f0'"
    exit /b
)

rem --- run the PowerShell engine next to this launcher -------------------------------
set "PS1=%~dp0blueStackDebloat.ps1"
if not exist "%PS1%" (
    echo [!] blueStackDebloat.ps1 not found next to this file.
    pause
    exit /b 1
)

powershell -NoProfile -ExecutionPolicy Bypass -File "%PS1%" %*

echo.
pause
endlocal
