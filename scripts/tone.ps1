param([string]$Path, [int]$Seconds = 8)
$rate = 44100
$n = $rate * $Seconds
$ms = New-Object System.IO.MemoryStream
$w = New-Object System.IO.BinaryWriter($ms)
$w.Write([Text.Encoding]::ASCII.GetBytes("RIFF")); $w.Write([int](36 + $n * 2)); $w.Write([Text.Encoding]::ASCII.GetBytes("WAVEfmt "))
$w.Write([int]16); $w.Write([int16]1); $w.Write([int16]1); $w.Write([int]$rate); $w.Write([int]($rate * 2)); $w.Write([int16]2); $w.Write([int16]16)
$w.Write([Text.Encoding]::ASCII.GetBytes("data")); $w.Write([int]($n * 2))
for ($i = 0; $i -lt $n; $i++) { $w.Write([int16](12000 * [math]::Sin(2 * [math]::PI * 440 * $i / $rate))) }
[IO.File]::WriteAllBytes($Path, $ms.ToArray())
