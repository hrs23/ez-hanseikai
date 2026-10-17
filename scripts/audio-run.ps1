$root = Join-Path $env:USERPROFILE "ezh\src-tauri"
$exe = Join-Path $root "target\release\examples\audio.exe"
$out = Join-Path $root "audio.log"
Remove-Item $out -ErrorAction SilentlyContinue

$wav = Join-Path $root "tone.wav"
& (Join-Path $PSScriptRoot "tone.ps1") -Path $wav

$player = Start-Process powershell -WindowStyle Hidden -PassThru -ArgumentList "-NoProfile", "-Command", "(New-Object Media.SoundPlayer '$wav').PlaySync()"
$silent = Start-Process powershell -WindowStyle Hidden -PassThru -ArgumentList "-NoProfile", "-Command", "Start-Sleep 30"
Start-Sleep 2
"playing process:" | Out-File $out
& $exe $player.Id *>> $out
"silent process:" | Out-File $out -Append
& $exe $silent.Id *>> $out
Stop-Process -Id $player.Id, $silent.Id -Force -ErrorAction SilentlyContinue
