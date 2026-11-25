import fs from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = fileURLToPath(new URL('../', import.meta.url))
const release = path.join(root, 'release-pack')
const name = process.platform === 'win32' ? 'dramlo-server.exe' : 'dramlo-server'
const binary = path.join(root, 'dramlo-server', 'target', 'release', name)
await fs.access(binary)
await fs.access(path.join(root, 'dist', 'index.html'))
await fs.mkdir(release, { recursive: true })
const directory = await fs.mkdtemp(path.join(release, 'Dramlo-Web-'))
await fs.copyFile(binary, path.join(directory, name))
await fs.chmod(path.join(directory, name), 0o755)
await fs.cp(path.join(root, 'dist'), path.join(directory, 'dist'), { recursive: true })
if (process.platform === 'win32') {
  await fs.copyFile(path.join(root, 'scripts', 'setup-ffmpeg.cmd'), path.join(directory, 'get-ffmpeg.cmd'))
  await fs.writeFile(path.join(directory, 'start.cmd'), [
    '@echo off',
    'setlocal',
    'set "DRAMLO_PACKAGE_WORKSPACE=%~dp0projects"',
    'if not "%~1"=="" set "DRAMLO_PACKAGE_WORKSPACE=%~f1"',
    'set "DRAMLO_PACKAGE_PORT=38042"',
    'if not "%~2"=="" set "DRAMLO_PACKAGE_PORT=%~2"',
    'if not defined DRAMLO_FFMPEG_BIN for /f "delims=" %%i in (\'where.exe ffmpeg.exe 2^>nul\') do if not defined DRAMLO_FFMPEG_BIN set "DRAMLO_FFMPEG_BIN=%%i"',
    '"%~dp0dramlo-server.exe" "%DRAMLO_PACKAGE_WORKSPACE%" "%~dp0dist" "%DRAMLO_PACKAGE_PORT%" --token-from-launcher',
    ''
  ].join('\r\n'))
}
console.log(directory)
