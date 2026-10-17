$ErrorActionPreference = "Stop"
$env:Path += ";$env:USERPROFILE\.cargo\bin"
$root = Join-Path $env:USERPROFILE "ezh\src-tauri"
Set-Location $root
cargo build --release --example record
if ($LASTEXITCODE -ne 0) { exit 1 }
$out = Join-Path $root "record.log"
Remove-Item $out -ErrorAction SilentlyContinue
$bat = Join-Path $root "record.cmd"
$run = Join-Path $root "record-run.log"
Remove-Item $run -ErrorAction SilentlyContinue
Set-Content $bat "powershell -NoProfile -ExecutionPolicy Bypass -File `"$env:USERPROFILE\record-run.ps1`" > `"$run`" 2>&1" -Encoding ASCII
schtasks /Create /TN ezh-record /TR $bat /SC ONCE /ST 00:00 /IT /F | Out-Null
schtasks /Run /TN ezh-record | Out-Null
$wait = 0
while (-not ((Test-Path $out) -and (Select-String -Path $out -Pattern "gpu engine" -Quiet)) -and $wait -lt 60) { Start-Sleep 2; $wait += 2 }
schtasks /Delete /TN ezh-record /F | Out-Null
if (Test-Path $run) { Get-Content $run }
Get-Content $out
$err = Join-Path $root "record.err"
if ((Test-Path $err) -and (Get-Item $err).Length -gt 0) { "stderr:"; Get-Content $err }
