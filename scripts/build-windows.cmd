@echo off
setlocal

pushd "%~dp0.." || exit /b 1

where npm >nul 2>nul || (
  echo Node.js was not found. Run scripts\setup-windows-dev.ps1 first.
  popd
  exit /b 1
)
where cargo >nul 2>nul || (
  echo Rust was not found. Run scripts\setup-windows-dev.ps1 first.
  popd
  exit /b 1
)

powershell -NoLogo -NoProfile -ExecutionPolicy Bypass -File "scripts\prepare-windows-assets.ps1"
if errorlevel 1 (
  popd
  exit /b 1
)
call npm ci
if errorlevel 1 (
  popd
  exit /b 1
)
call npm run tauri build -- --config src-tauri/tauri.local.conf.json
set EXIT_CODE=%ERRORLEVEL%

if %EXIT_CODE%==0 (
  echo Installer: src-tauri\target\release\bundle\nsis\
)
popd
exit /b %EXIT_CODE%
