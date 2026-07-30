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

if not exist "src-tauri\resources\xray\xray.exe" (
  powershell -NoLogo -NoProfile -ExecutionPolicy Bypass -File "scripts\prepare-windows-assets.ps1" || (
    popd
    exit /b 1
  )
)

call npm ci
if errorlevel 1 (
  popd
  exit /b 1
)

call npm run tauri dev
set EXIT_CODE=%ERRORLEVEL%
popd
exit /b %EXIT_CODE%
