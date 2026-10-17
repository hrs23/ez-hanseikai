$ErrorActionPreference = "Stop"
$env:Path += ";$env:USERPROFILE\.cargo\bin"
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root
$bin = Join-Path $root "src-tauri\binaries"
New-Item -ItemType Directory -Force $bin | Out-Null
Copy-Item (Get-Command ffmpeg).Source (Join-Path $bin "ffmpeg-x86_64-pc-windows-msvc.exe") -Force
npm install --no-audit --no-fund | Out-Null
$offline = Join-Path $root "src-tauri\tauri.offline.conf.json"
'{"bundle":{"externalBin":["binaries/ffmpeg"],"createUpdaterArtifacts":false}}' | Set-Content $offline -Encoding ASCII
npx tauri build --config $offline
if ($LASTEXITCODE -ne 0) { exit 1 }
$setup = Get-ChildItem (Join-Path $root "src-tauri\target\release\bundle\nsis\*-setup.exe") | Sort-Object LastWriteTime | Select-Object -Last 1
$hash = (Get-FileHash $setup.FullName -Algorithm SHA256).Hash.ToLower()
"$hash  $($setup.Name)" | Set-Content "$($setup.FullName).sha256" -Encoding ASCII
$setup.FullName
