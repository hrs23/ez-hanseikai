$ErrorActionPreference = "Stop"
$env:Path += ";$env:USERPROFILE\.cargo\bin"
$root = Join-Path $env:USERPROFILE "ezh\src-tauri"
Set-Location $root
cargo build --release --example capture
if ($LASTEXITCODE -ne 0) { exit 1 }
$exe = Join-Path $root "target\release\examples\capture.exe"
$out = Join-Path $root "capture.log"
Remove-Item $out, "$root\capture.mp4" -ErrorAction SilentlyContinue
$bat = Join-Path $root "capture.cmd"
Set-Content $bat "cd /d $root`r`n`"$exe`" > `"$out`" 2>&1" -Encoding ASCII
schtasks /Create /TN ezh-capture /TR $bat /SC ONCE /ST 00:00 /IT /F | Out-Null
schtasks /Run /TN ezh-capture | Out-Null
Start-Sleep 12
schtasks /Delete /TN ezh-capture /F | Out-Null
Get-Content $out
Get-Item "$root\capture.mp4" | Select-Object Name, Length | Format-Table -AutoSize | Out-String
