param([string]$AppArgs = "")
$ErrorActionPreference = "Stop"
$env:Path += ";$env:USERPROFILE\.cargo\bin"
$root = Join-Path $env:USERPROFILE "ezh"
Get-Process ez-hanseikai -ErrorAction SilentlyContinue | Stop-Process -Force
New-Item -ItemType Directory -Force $root | Out-Null
Get-ChildItem $root -Force | Where-Object { $_.Name -ne "node_modules" } | ForEach-Object {
    if ($_.Name -eq "src-tauri") {
        Get-ChildItem $_.FullName -Force | Where-Object { $_.Name -ne "target" } | Remove-Item -Recurse -Force
    } else {
        Remove-Item $_.FullName -Recurse -Force
    }
}
Expand-Archive (Join-Path $env:USERPROFILE "ezh.zip") $root -Force
Set-Location $root
npm install --no-audit --no-fund | Out-Null
$bin = Join-Path $root "src-tauri\binaries"
New-Item -ItemType Directory -Force $bin | Out-Null
Copy-Item (Get-Command ffmpeg).Source (Join-Path $bin "ffmpeg-x86_64-pc-windows-msvc.exe") -Force
npx tauri build
if ($LASTEXITCODE -ne 0) { exit 1 }
$exe = Join-Path $root "src-tauri\target\release\ez-hanseikai.exe"
schtasks /Create /TN ezh-run /TR "`"$exe`" $AppArgs" /SC ONCE /ST 00:00 /IT /F | Out-Null
schtasks /Run /TN ezh-run | Out-Null
Start-Sleep 5
schtasks /Delete /TN ezh-run /F | Out-Null
Get-Process ez-hanseikai | Select-Object Name, SessionId | Format-Table -AutoSize | Out-String
