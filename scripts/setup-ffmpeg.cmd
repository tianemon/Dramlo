@echo off
setlocal
chcp 65001 >nul
set "DIR=%~dp0ffmpeg"
set "GZ=%TEMP%\ffmpeg-win32-x64.gz"
set "ZIP=%TEMP%\ffmpeg-essentials.zip"
set "TMP=%TEMP%\ffmpeg-extract"

if exist "%DIR%\ffmpeg.exe" (
  echo 已存在 %DIR%\ffmpeg.exe，如需重装先删除该文件。
  goto :check
)
if not exist "%DIR%" mkdir "%DIR%"

echo 从国内镜像下载 ffmpeg，约 29 MB…
powershell -NoProfile -ExecutionPolicy Bypass -Command ^
  "$ProgressPreference='SilentlyContinue'; Invoke-WebRequest -Uri 'https://cdn.npmmirror.com/binaries/ffmpeg-static/b6.0/ffmpeg-win32-x64.gz' -OutFile '%GZ%'"
if exist "%GZ%" (
  echo 正在解压…
  powershell -NoProfile -ExecutionPolicy Bypass -Command ^
    "$in=[IO.File]::OpenRead('%GZ%'); $out=[IO.File]::Create('%DIR%\ffmpeg.exe'); $gz=New-Object IO.Compression.GzipStream($in,[IO.Compression.CompressionMode]::Decompress); $gz.CopyTo($out); $gz.Dispose(); $out.Dispose(); $in.Dispose()"
  del "%GZ%" 2>nul
)

if not exist "%DIR%\ffmpeg.exe" (
  echo 国内镜像不可用，改用官方源，约 120 MB…
  powershell -NoProfile -ExecutionPolicy Bypass -Command ^
    "$ProgressPreference='SilentlyContinue'; Invoke-WebRequest -Uri 'https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip' -OutFile '%ZIP%'"
  if exist "%ZIP%" (
    powershell -NoProfile -ExecutionPolicy Bypass -Command ^
      "Expand-Archive -Path '%ZIP%' -DestinationPath '%TMP%' -Force"
    for /r "%TMP%" %%f in (ffmpeg.exe) do if not exist "%DIR%\ffmpeg.exe" copy /y "%%f" "%DIR%\ffmpeg.exe" >nul
    rmdir /s /q "%TMP%" 2>nul
    del "%ZIP%" 2>nul
  )
)

if not exist "%DIR%\ffmpeg.exe" (
  echo 下载失败。可手动下载后把 ffmpeg.exe 放到 %DIR%\ 下。
  exit /b 1
)
echo 已放置：%DIR%\ffmpeg.exe

:check
"%DIR%\ffmpeg.exe" -version | findstr /i "ffmpeg version" >nul && echo ffmpeg 可用。 || echo ffmpeg 无法运行，请检查文件完整性。
