@echo off
setlocal

set "PORT=%~1"
if "%PORT%"=="" set "PORT=8765"

for %%I in ("%~dp0..\..") do set "REPO_ROOT=%%~fI"
set "HOST_PATH=apps\z-audio-wclap-host"
set "URL=http://127.0.0.1:%PORT%/%HOST_PATH:\=/%/"
set "OPEN_URL=%URL%?v=%RANDOM%%RANDOM%"

where py >nul 2>nul
if not errorlevel 1 (
    set "PYTHON_CMD=py -3"
) else (
    where python >nul 2>nul
    if errorlevel 1 (
        echo Python 3 was not found. Install Python or add it to PATH.
        pause
        exit /b 1
    )
    set "PYTHON_CMD=python"
)

cd /d "%REPO_ROOT%" || exit /b 1

echo Starting Z Audio WebCLAP Host
echo URL: %URL%
echo.
echo Tip: run "cargo xtask bundle-webclap --release" first to refresh target\webclap bundles.
echo Press Ctrl+C in this window to stop the host.
echo.

start "" "%OPEN_URL%"
%PYTHON_CMD% "%HOST_PATH%\server.py" "%PORT%"
