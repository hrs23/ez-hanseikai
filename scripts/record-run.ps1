param([int]$TargetPid = 0)
$root = Join-Path $env:USERPROFILE "ezh\src-tauri"
$exe = Join-Path $root "target\release\examples\record.exe"
$out = Join-Path $root "record.log"
Remove-Item $out, (Join-Path $root "record.mp4") -ErrorAction SilentlyContinue

$player = $null
if ($TargetPid -eq 0) {
    $wav = Join-Path $root "tone.wav"
    & (Join-Path $PSScriptRoot "tone.ps1") -Path $wav -Seconds 20
    $player = Start-Process powershell -WindowStyle Hidden -PassThru -ArgumentList "-NoProfile", "-Command", "Start-Sleep 2; (New-Object Media.SoundPlayer '$wav').PlaySync()"
    $TargetPid = $player.Id
}
$rec = Start-Process $exe -ArgumentList $TargetPid, 10 -WorkingDirectory $root -PassThru -WindowStyle Hidden -RedirectStandardOutput $out -RedirectStandardError (Join-Path $root "record.err")

$cpu = 0
$gpu = @{}
$samples = 0
while (-not $rec.HasExited) {
    try { $cpu = $rec.TotalProcessorTime.TotalSeconds } catch {}
    try {
        $c = (Get-Counter '\GPU Engine(*)\Utilization Percentage' -ErrorAction Stop).CounterSamples | Where-Object { $_.InstanceName -like "pid_$($rec.Id)_*" }
        $byType = @{}
        foreach ($s in $c) {
            $type = ($s.InstanceName -split "engtype_")[1]
            $byType[$type] += $s.CookedValue
        }
        foreach ($k in $byType.Keys) { $gpu[$k] += $byType[$k] }
        $samples++
    } catch {}
}
if ($player) { Stop-Process -Id $player.Id -Force -ErrorAction SilentlyContinue }
"cpu seconds: {0:N1} over 10 s wall, {1} logical cores" -f $cpu, [Environment]::ProcessorCount | Out-File $out -Append
"gpu engine average utilization of the recorder (%), $samples samples:" | Out-File $out -Append
foreach ($k in ($gpu.Keys | Sort-Object)) { "  {0}: {1:N1}" -f $k, ($gpu[$k] / [math]::Max($samples, 1)) | Out-File $out -Append }
