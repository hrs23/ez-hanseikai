$ErrorActionPreference = "Stop"
$env:Path += ";$env:USERPROFILE\.cargo\bin"
$root = Join-Path $env:USERPROFILE "ezh\src-tauri"
Set-Location $root
cargo build --release --example audio
if ($LASTEXITCODE -ne 0) { exit 1 }
$out = Join-Path $root "audio.log"
Remove-Item $out -ErrorAction SilentlyContinue
$bat = Join-Path $root "audio.cmd"
Set-Content $bat "powershell -NoProfile -ExecutionPolicy Bypass -File `"$env:USERPROFILE\audio-run.ps1`"" -Encoding ASCII
schtasks /Create /TN ezh-audio /TR $bat /SC ONCE /ST 00:00 /IT /F | Out-Null
schtasks /Run /TN ezh-audio | Out-Null
Start-Sleep 25
schtasks /Delete /TN ezh-audio /F | Out-Null
Get-Content $out
