# bootguard installer: downloads the latest release exe to %LOCALAPPDATA%\bootguard and adds it to the user PATH.
$ErrorActionPreference = 'Stop'
$dir = Join-Path $env:LOCALAPPDATA 'bootguard'
New-Item -ItemType Directory -Force $dir | Out-Null
$url = 'https://github.com/Michaelunkai/bootguard-rs/releases/latest/download/bootguard.exe'
Invoke-WebRequest $url -OutFile (Join-Path $dir 'bootguard.exe') -UseBasicParsing
$p = [Environment]::GetEnvironmentVariable('Path','User')
if (($p -split ';') -notcontains $dir) { [Environment]::SetEnvironmentVariable('Path', "$p;$dir", 'User') }
Write-Host "Installed: $dir\bootguard.exe  (open a new terminal and run: bootguard.exe --check-only)"
