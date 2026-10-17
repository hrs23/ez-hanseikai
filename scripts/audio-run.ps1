$root = Join-Path $env:USERPROFILE "ezh\src-tauri"
$exe = Join-Path $root "target\release\examples\audio.exe"
$out = Join-Path $root "audio.log"
Remove-Item $out -ErrorAction SilentlyContinue

$rate = 44100
$n = $rate * 8
$ms = New-Object System.IO.MemoryStream
$w = New-Object System.IO.BinaryWriter($ms)
$w.Write([Text.Encoding]::ASCII.GetBytes("RIFF")); $w.Write([int](36 + $n * 2)); $w.Write([Text.Encoding]::ASCII.GetBytes("WAVEfmt "))
$w.Write([int]16); $w.Write([int16]1); $w.Write([int16]1); $w.Write([int]$rate); $w.Write([int]($rate * 2)); $w.Write([int16]2); $w.Write([int16]16)
$w.Write([Text.Encoding]::ASCII.GetBytes("data")); $w.Write([int]($n * 2))
for ($i = 0; $i -lt $n; $i++) { $w.Write([int16](12000 * [math]::Sin(2 * [math]::PI * 440 * $i / $rate))) }
$wav = Join-Path $root "tone.wav"
[IO.File]::WriteAllBytes($wav, $ms.ToArray())

$player = Start-Process powershell -WindowStyle Hidden -PassThru -ArgumentList "-NoProfile", "-Command", "(New-Object Media.SoundPlayer '$wav').PlaySync()"
$silent = Start-Process powershell -WindowStyle Hidden -PassThru -ArgumentList "-NoProfile", "-Command", "Start-Sleep 30"
Start-Sleep 2
"playing process:" | Out-File $out
& $exe $player.Id *>> $out
"silent process:" | Out-File $out -Append
& $exe $silent.Id *>> $out
Stop-Process -Id $player.Id, $silent.Id -Force -ErrorAction SilentlyContinue
