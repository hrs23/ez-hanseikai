use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use iroh::endpoint::Connection;
use iroh::Watcher;
use regex::Regex;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::EnvFilter;

use crate::{ffmpeg, room, short, Core, NO_WINDOW};

const MAX_LOG: u64 = 2 << 20;
const EVENTS_KEPT: usize = 60;
const REPORT_LINES: usize = 400;

#[derive(Clone)]
struct RollingFile {
    path: PathBuf,
    file: Arc<Mutex<(File, u64)>>,
}

impl RollingFile {
    fn open(path: PathBuf) -> std::io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let len = file.metadata().map(|m| m.len()).unwrap_or(0);
        Ok(Self { path, file: Arc::new(Mutex::new((file, len))) })
    }
}

impl Write for RollingFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut file = self.file.lock().unwrap();
        if file.1 > MAX_LOG {
            let _ = fs::rename(&self.path, self.path.with_extension("old.txt"));
            *file = (OpenOptions::new().create(true).append(true).open(&self.path)?, 0);
        }
        let n = file.0.write(buf)?;
        file.1 += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.lock().unwrap().0.flush()
    }
}

impl<'a> MakeWriter<'a> for RollingFile {
    type Writer = RollingFile;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

pub fn init(dir: &Path) {
    let Ok(writer) = RollingFile::open(dir.join("log.txt")) else { return };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn,ez_hanseikai=debug,iroh=info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(false)
        .with_timer(tracing_subscriber::fmt::time::uptime())
        .init();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("panic: {info}\n{}", std::backtrace::Backtrace::force_capture());
        previous(info);
    }));
}

fn describe(conn: &Connection, label: &str, out: &mut String) {
    out.push_str(&format!("{label} connection to {}\n", conn.remote_id().fmt_short()));
    for path in conn.paths().iter() {
        let stats = path.stats();
        let kind = if path.is_relay() { "relay" } else if path.is_ip() { "direct" } else { "other" };
        out.push_str(&format!(
            "  path {kind}{} remote {:?} rtt {:?} sent {} B received {} B mtu {}\n",
            if path.is_selected() { " (selected)" } else { "" },
            path.remote_addr(),
            stats.rtt,
            stats.udp_tx.bytes,
            stats.udp_rx.bytes,
            stats.current_mtu
        ));
    }
    let stats = conn.stats();
    out.push_str(&format!("  total sent {} B received {} B lost packets {}\n", stats.udp_tx.bytes, stats.udp_rx.bytes, stats.lost_packets));
}

pub fn text(core: &Core) -> String {
    let mut out = String::new();
    out.push_str(&format!("encoder {} ({})\n", ffmpeg::encoder(), ffmpeg::path().display()));
    out.push_str(&format!("endpoint id {}\n", core.ep.id()));
    out.push_str(&format!("bound sockets {:?}\n", core.ep.bound_sockets()));
    out.push_str(&format!("address {:?}\n", core.ep.addr()));
    out.push_str(&format!("home relay {:?}\n", core.ep.home_relay_status().get()));
    out.push_str(&format!("net report {:?}\n", core.ep.net_report().get()));
    let peers: Vec<(String, Option<Connection>, Option<Connection>)> =
        core.peers.lock().unwrap().iter().map(|p| (p.id.to_string(), p.conn.clone(), p.incoming.clone())).collect();
    for (id, conn, incoming) in peers {
        out.push_str(&format!("peer {id}\n"));
        for (label, conn) in [("outgoing", conn), ("incoming", incoming)] {
            match conn {
                Some(conn) => describe(&conn, label, &mut out),
                None => out.push_str(&format!("{label} connection: none\n")),
            }
        }
    }
    let room = core.active_room_id();
    let logs = room::logs(core, &room);
    for set in room::sets_json(core, &room, &logs, &[]) {
        out.push_str(&format!("set {} {}\n", set["id"].as_str().unwrap_or_default(), set["status"].as_str().unwrap_or_default()));
        for p in set["players"].as_array().into_iter().flatten() {
            out.push_str(&format!(
                "  {} have {} of {} B in {} parts verified by {}\n",
                short(p["owner"].as_str().unwrap_or_default()),
                p["have"],
                p["total"],
                p["parts"].as_array().map_or(0, |a| a.len()),
                p["verified_by"].as_array().into_iter().flatten().filter_map(|k| k.as_str()).map(short).collect::<Vec<_>>().join(", ")
            ));
        }
    }
    out.push_str("events\n");
    for line in core.events.lock().unwrap().iter() {
        out.push_str(&format!("  {line}\n"));
    }
    out
}

pub fn event(core: &Core, since: Instant, line: String) {
    let line = format!("+{:.2}s {line}", since.elapsed().as_secs_f64());
    tracing::info!("{line}");
    let mut events = core.events.lock().unwrap();
    events.push_back(line);
    if events.len() > EVENTS_KEPT {
        events.pop_front();
    }
}

pub fn command_line(program: &str, args: &[&str]) -> String {
    Command::new(program)
        .args(args)
        .creation_flags(NO_WINDOW)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|e| format!("unavailable: {e}"))
}

fn redact(text: &str) -> String {
    let patterns = [
        (r"(?i)[a-z]:\\users\\[^\\]+", r"C:\Users\<user>"),
        (r"https?://[^\s/\x22,)]+", "<host>"),
        (r"\b\d{1,3}(?:\.\d{1,3}){3}\b", "<ip>"),
        (r"\[?[0-9a-fA-F]{0,4}(?::[0-9a-fA-F]{0,4}){3,7}\]?", "<ip6>"),
        (r"\b[0-9a-f]{64}\b", "<id>"),
    ];
    let mut text = text.to_string();
    for (pattern, replacement) in patterns {
        text = Regex::new(pattern).unwrap().replace_all(&text, replacement).into_owned();
    }
    text
}

pub fn report(core: &Core, include_addresses: bool, check: &str) -> std::io::Result<PathBuf> {
    let log = fs::read_to_string(core.dir.join("log.txt")).unwrap_or_default();
    let recent: Vec<&str> = log.lines().rev().take(REPORT_LINES).collect();
    let recent: Vec<&str> = recent.into_iter().rev().collect();
    let mut out = String::new();
    out.push_str(&format!("ez-hanseikai {} on {} {}\n", crate::VERSION, std::env::consts::OS, command_line("cmd", &["/c", "ver"])));
    out.push_str(&format!("gpu {}\n", command_line("powershell", &["-NoProfile", "-Command", "(Get-CimInstance Win32_VideoController).Name -join ', '"])));
    out.push_str(&format!("previous version {}\n", if core.previous_version.is_empty() { "none" } else { &core.previous_version }));
    if include_addresses {
        out.push_str(&format!("name {}\n", core.name.lock().unwrap()));
    }
    out.push_str(&format!("encoder {}\n", ffmpeg::encoder()));
    out.push_str(&format!("ffmpeg {}\n\n", command_line(&ffmpeg::path().to_string_lossy(), &["-version"]).lines().next().unwrap_or("")));
    let infos: Vec<String> = core.peers.lock().unwrap().iter().map(|p| format!("{} {:?}", short(p.id), p.info)).collect();
    out.push_str(&format!("peers {}\n\nenvironment check\n{check}\n\n", infos.join(", ")));
    out.push_str(&text(core));
    out.push_str("\nrecent log\n");
    out.push_str(&recent.join("\n"));
    let out = if include_addresses { out } else { redact(&out) };
    let path = core.dir.join("report.txt");
    fs::write(&path, out)?;
    Ok(path)
}
