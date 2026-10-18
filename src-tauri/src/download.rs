use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

use crate::proc;

const TIMEOUT_MS: &str = "60000";
const WAIT: Duration = Duration::from_secs(6 * 3600);
const SCRIPT: &str = r#"
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$ErrorActionPreference = 'Stop'
trap { [Console]::Error.WriteLine($_.Exception.Message); exit 1 }
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor 3072
$part = $env:EZH_PART
$out = $env:EZH_OUT
try {
  $request = [Net.HttpWebRequest]::Create($env:EZH_URL)
  $request.Timeout = [int]$env:EZH_TIMEOUT
  $request.ReadWriteTimeout = [int]$env:EZH_TIMEOUT
  $response = $request.GetResponse()
  $body = $response.GetResponseStream()
  $file = [IO.File]::Create($part)
  try { $body.CopyTo($file) } finally { $file.Dispose(); $body.Dispose(); $response.Dispose() }
  if ((Get-Item -LiteralPath $part).Length -ne [int64]$env:EZH_SIZE) { throw 'The download has the wrong size.' }
  if ((Get-FileHash -LiteralPath $part -Algorithm SHA256).Hash.ToLower() -ne $env:EZH_SHA) { throw 'The download has the wrong checksum.' }
  if ($env:EZH_PICK -eq '') {
    Move-Item -Force -LiteralPath $part -Destination $out
  } else {
    [IO.Directory]::CreateDirectory($out) | Out-Null
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $patterns = $env:EZH_PICK -split '\|'
    $zip = [IO.Compression.ZipFile]::OpenRead($part)
    try {
      $entries = @($zip.Entries | Where-Object { $name = $_.Name; $name -and @($patterns | Where-Object { $name -like $_ }).Count })
      $first = @($entries | Where-Object { $_.Name -eq $patterns[0] })
      if (-not $first.Count) { throw "$($patterns[0]) is missing from the download." }
      foreach ($entry in @($entries | Where-Object { $_.Name -ne $patterns[0] }) + $first) {
        $target = Join-Path $out $entry.Name
        [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, "$target.part", $true)
        Move-Item -Force -LiteralPath "$target.part" -Destination $target
      }
    } finally { $zip.Dispose() }
  }
} finally { Remove-Item -LiteralPath $part -ErrorAction SilentlyContinue }
"#;

pub struct Download {
    pub url: String,
    pub sha256: &'static str,
    pub size: u64,
    pub dest: String,
    pub pick: &'static [&'static str],
}

static BUSY: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

pub fn present(dir: &Path, d: &Download) -> bool {
    match d.pick.first() {
        Some(first) => dir.join(&d.dest).join(first).exists(),
        None => dir.join(&d.dest).exists(),
    }
}

fn part_name(url: &str) -> String {
    let name = url.rsplit('/').next().unwrap_or_default();
    let safe: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' { c } else { '_' }).collect();
    format!("{safe}.part")
}

struct Busy(PathBuf);

impl Drop for Busy {
    fn drop(&mut self) {
        BUSY.lock().unwrap().retain(|p| *p != self.0);
    }
}

pub fn fetch(dir: &Path, d: &Download) -> Result<(), String> {
    let target = dir.join(&d.dest);
    let _busy = {
        let mut busy = BUSY.lock().unwrap();
        if busy.contains(&target) {
            return Err("Already downloading.".into());
        }
        busy.push(target.clone());
        Busy(target.clone())
    };
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut command = Command::new("powershell");
    command
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", SCRIPT])
        .env("EZH_URL", &d.url)
        .env("EZH_SHA", d.sha256)
        .env("EZH_SIZE", d.size.to_string())
        .env("EZH_PART", dir.join(part_name(&d.url)))
        .env("EZH_OUT", &target)
        .env("EZH_PICK", d.pick.join("|"))
        .env("EZH_TIMEOUT", TIMEOUT_MS);
    proc::run(&mut command, 0, WAIT).map(|_| ()).map_err(|e| e.lines().last().unwrap_or("download failed").trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_names_are_plain_and_follow_the_url() {
        assert_eq!(part_name("https://example.com/a/ffmpeg-8.1-essentials_build.zip"), "ffmpeg-8.1-essentials_build.zip.part");
        assert_eq!(part_name("https://example.com/x/it's café.bin"), "it_s_caf_.bin.part");
    }
}
