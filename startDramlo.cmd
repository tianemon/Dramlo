@echo off
setlocal
cd /d "%~dp0"
if "%~1"=="" (
  echo Usage: startDramlo.cmd ^<project-workspace^> [port]
  exit /b 2
)
if not exist "dist\index.html" (
  echo Build the Vue frontend with npm run build first.
  exit /b 2
)
if not exist "dramlo-server\target\release\dramlo-server.exe" (
  echo Build the service with cargo build --release --manifest-path dramlo-server\Cargo.toml first.
  exit /b 2
)
"dramlo-server\target\release\dramlo-server.exe" "%~f1" "%~dp0dist" %2
