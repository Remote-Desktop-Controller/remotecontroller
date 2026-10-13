@echo off
title RDC Maestro Launcher
echo ========================================================
echo           INICIANDO RDC MAESTRO (DESKTOP)
echo ========================================================
echo.
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
cd /d "%~dp0apps\rdc-maestro"

echo [1/2] Verificando dependencias...
echo [2/2] Abrindo interface nativa do Tauri...
echo.
call npm run tauri dev
pause
