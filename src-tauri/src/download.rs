use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::proc;

const WAIT: Duration = Duration::from_secs(6 * 3600);
const WATCH_EVERY: Duration = Duration::from_millis(500);
const SCRIPT: &str = r#"
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$ErrorActionPreference = 'Stop'
trap { [Console]::Error.WriteLine($_.Exception.Message); exit 1 }
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor 3072
$part = $env:EZH_PART
$out = $env:EZH_OUT
try {
  $request = [Net.HttpWebRequest]::Create($env:EZH_URL)
  $request.Timeout = 60000
  $request.ReadWriteTimeout = 60000
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
} finally { if (Test-Path -LiteralPath $part) { Remove-Item -LiteralPath $part } }
exit 0
"#;

pub struct Download {
    pub url: String,
    pub sha256: &'static str,
    pub size: u64,
    pub dest: String,
    pub pick: &'static [&'static str],
}

static BUSY: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
static TRANSFERS: Mutex<BTreeMap<String, Transfer>> = Mutex::new(BTreeMap::new());

struct Transfer {
    done: u64,
    total: u64,
    started: Instant,
    hosts: BTreeSet<String>,
}

pub fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let host = rest.split(['/', '?', '#']).next()?.rsplit('@').next()?.split(':').next()?;
    (!host.is_empty()).then(|| host.to_lowercase())
}

pub fn active() -> Vec<Value> {
    TRANSFERS.lock().unwrap().iter().map(|(key, t)| json!({"key": key, "hosts": t.hosts})).collect()
}

pub fn pace(done: u64, total: u64, seconds: f64) -> (u64, Option<u64>) {
    if done == 0 || seconds <= 0.0 {
        return (0, None);
    }
    let speed = done as f64 / seconds;
    (speed as u64, Some((total.saturating_sub(done) as f64 / speed).ceil() as u64))
}

pub fn progress(key: &str) -> Value {
    let transfers = TRANSFERS.lock().unwrap();
    let Some(t) = transfers.get(key) else { return Value::Null };
    let (speed, left) = pace(t.done, t.total, t.started.elapsed().as_secs_f64());
    json!({"done": t.done, "total": t.total, "speed": speed, "left_s": left})
}

fn set_done(key: &str, done: u64) {
    if let Some(t) = TRANSFERS.lock().unwrap().get_mut(key) {
        t.done = done;
    }
}

pub fn fetch_group(key: &str, dir: &Path, downloads: &[Download]) -> Result<(), String> {
    let total = downloads.iter().map(|d| d.size).sum();
    {
        let mut transfers = TRANSFERS.lock().unwrap();
        if transfers.contains_key(key) {
            return Err("Already downloading.".into());
        }
        let hosts = downloads.iter().filter_map(|d| host_of(&d.url)).collect();
        transfers.insert(key.to_string(), Transfer { done: 0, total, started: Instant::now(), hosts });
    }
    let mut base = 0;
    let mut result = Ok(());
    for d in downloads {
        let stop = Arc::new(AtomicBool::new(false));
        let watcher = {
            let (stop, key, part) = (stop.clone(), key.to_string(), dir.join(part_name(&d.url)));
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    set_done(&key, base + std::fs::metadata(&part).map_or(0, |m| m.len()));
                    std::thread::sleep(WATCH_EVERY);
                }
            })
        };
        result = fetch(dir, d);
        stop.store(true, Ordering::Relaxed);
        let _ = watcher.join();
        if result.is_err() {
            break;
        }
        base += d.size;
        set_done(key, base);
    }
    TRANSFERS.lock().unwrap().remove(key);
    result
}

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
        .env("EZH_PICK", d.pick.join("|"));
    proc::run(&mut command, 0, WAIT).map(|_| ()).map_err(|e| e.lines().last().unwrap_or("download failed").trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pace_gives_speed_and_time_left() {
        assert_eq!(pace(0, 100, 5.0), (0, None));
        assert_eq!(pace(50, 100, 10.0), (5, Some(10)));
        assert_eq!(pace(100, 100, 10.0), (10, Some(0)));
        assert_eq!(pace(120, 100, 10.0).1, Some(0));
    }

    #[test]
    fn hosts_come_from_the_url_alone() {
        assert_eq!(host_of("https://Example.COM/a/b.zip?x=1").as_deref(), Some("example.com"));
        assert_eq!(host_of("https://user:pw@files.example.net:8443/x").as_deref(), Some("files.example.net"));
        assert_eq!(host_of("not a url"), None);
        assert_eq!(host_of("https:///x"), None);
    }

    #[test]
    fn part_names_are_plain_and_follow_the_url() {
        assert_eq!(part_name("https://example.com/a/ffmpeg-8.1-essentials_build.zip"), "ffmpeg-8.1-essentials_build.zip.part");
        assert_eq!(part_name("https://example.com/x/it's café.bin"), "it_s_caf_.bin.part");
    }
}
