param([switch]$Installer, [switch]$Test)
$ErrorActionPreference = "Stop"
$env:Path += ";$env:USERPROFILE\.cargo\bin"
$root = Join-Path $env:USERPROFILE "ezh"
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
if ($Test) {
    cargo test --manifest-path src-tauri/Cargo.toml
    exit $LASTEXITCODE
}
Get-Process ggi-hanseikai, ez-hanseikai -ErrorAction SilentlyContinue | Stop-Process -Force
if ($Installer) {
    & (Join-Path $root "scripts\build-installer.ps1")
} else {
    $release = Join-Path $root "src-tauri\target\release"
    New-Item -ItemType Directory -Force $release | Out-Null
    if (!(Test-Path (Join-Path $release "ffmpeg.exe"))) { Copy-Item (Get-Command ffmpeg).Source (Join-Path $release "ffmpeg.exe") }
    npm install --no-audit --no-fund | Out-Null
    npx tauri build --no-bundle
}
if ($LASTEXITCODE -ne 0) { exit 1 }
$exe = Join-Path $root "src-tauri\target\release\ggi-hanseikai.exe"
schtasks /Create /TN ezh-run /TR "`"$exe`"" /SC ONCE /ST 00:00 /IT /F | Out-Null
schtasks /Run /TN ezh-run | Out-Null
Start-Sleep 5
schtasks /Delete /TN ezh-run /F | Out-Null
Get-Process ggi-hanseikai | Select-Object Name, SessionId | Format-Table -AutoSize | Out-String
