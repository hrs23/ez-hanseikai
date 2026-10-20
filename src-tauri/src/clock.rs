use std::net::{ToSocketAddrs, UdpSocket};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

pub const HOST: &str = "time.cloudflare.com";
const CLOCK_STEP_US: i64 = 500_000;
const PORT: u16 = 123;
const SAMPLES: usize = 3;
const TIMEOUT: Duration = Duration::from_secs(3);
const EVERY: Duration = Duration::from_secs(3600);
const RETRY: Duration = Duration::from_secs(60);
const WAKE: Duration = Duration::from_secs(1);
const NTP_EPOCH_S: i64 = 2_208_988_800;
const PACKET: usize = 48;

#[derive(Clone, Copy)]
struct Measured {
    offset_us: i64,
    rtt_us: i64,
    at_us: i64,
    at: Instant,
}

static STATE: Mutex<(Option<Measured>, Option<Instant>)> = Mutex::new((None, None));

pub fn pc_us() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_micros() as i64
}

fn moved(at_us: i64, now_us: i64, elapsed: Duration) -> i64 {
    now_us - at_us - elapsed.as_micros() as i64
}

pub fn stepped(at_us: i64, now_us: i64, elapsed: Duration) -> bool {
    moved(at_us, now_us, elapsed).abs() > CLOCK_STEP_US
}

fn to_ntp(us: i64) -> u64 {
    let (s, frac) = (us.div_euclid(1_000_000) + NTP_EPOCH_S, us.rem_euclid(1_000_000) as u64);
    ((s as u64) << 32) | ((frac << 32) / 1_000_000)
}

fn from_ntp(v: u64) -> i64 {
    ((v >> 32) as i64 - NTP_EPOCH_S) * 1_000_000 + (((v & 0xffff_ffff) * 1_000_000) >> 32) as i64
}

fn request(t0_us: i64) -> [u8; PACKET] {
    let mut out = [0u8; PACKET];
    out[0] = (4 << 3) | 3;
    out[40..].copy_from_slice(&to_ntp(t0_us).to_be_bytes());
    out
}

fn reply_times(bytes: &[u8], t0_us: i64) -> Option<(i64, i64)> {
    let word = |at: usize| Some(u64::from_be_bytes(bytes.get(at..at + 8)?.try_into().ok()?));
    if bytes.len() < PACKET || bytes[0] & 7 != 4 || !(1..=15).contains(&bytes[1]) || word(24)? != to_ntp(t0_us) {
        return None;
    }
    Some((from_ntp(word(32)?), from_ntp(word(40)?)))
}

fn offset_and_rtt(t0: i64, t1: i64, t2: i64, t3: i64) -> (i64, i64) {
    (((t1 - t0) + (t2 - t3)) / 2, (t3 - t0) - (t2 - t1))
}

fn server() -> String {
    std::env::var("EZH_NTP").ok().filter(|_| crate::test_mode()).unwrap_or_else(|| format!("{HOST}:{PORT}"))
}

fn sample(socket: &UdpSocket) -> Result<Measured, String> {
    let t0 = pc_us();
    socket.send(&request(t0)).map_err(|e| e.to_string())?;
    let mut buf = [0u8; 512];
    let n = socket.recv(&mut buf).map_err(|e| e.to_string())?;
    let (t3, at) = (pc_us(), Instant::now());
    let (t1, t2) = reply_times(&buf[..n], t0).ok_or("not a time answer")?;
    let (offset_us, rtt_us) = offset_and_rtt(t0, t1, t2, t3);
    Ok(Measured { offset_us, rtt_us, at_us: t3, at })
}

fn measure(server: &str) -> Result<Measured, String> {
    let addr = server.to_socket_addrs().map_err(|e| e.to_string())?.next().ok_or("no address")?;
    let socket = UdpSocket::bind(if addr.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" }).map_err(|e| e.to_string())?;
    socket.set_read_timeout(Some(TIMEOUT)).and_then(|_| socket.connect(addr)).map_err(|e| e.to_string())?;
    let mut best: Option<Measured> = None;
    for _ in 0..SAMPLES {
        match sample(&socket) {
            Ok(m) => best = best.filter(|b| b.rtt_us <= m.rtt_us).or(Some(m)),
            Err(e) => return best.ok_or(e),
        }
    }
    best.ok_or_else(|| "no answer".into())
}

fn corrected(m: &Measured, now_pc_us: i64) -> i64 {
    m.offset_us - moved(m.at_us, now_pc_us, m.at.elapsed())
}

fn due(last: Option<&Measured>, failed: Option<Instant>, now_pc_us: i64) -> bool {
    match (failed, last) {
        (Some(f), _) => f.elapsed() >= RETRY,
        (None, None) => true,
        (None, Some(m)) => m.at.elapsed() >= EVERY || stepped(m.at_us, now_pc_us, m.at.elapsed()),
    }
}

pub fn offset_us() -> i64 {
    STATE.lock().unwrap().0.as_ref().map_or(0, |m| corrected(m, pc_us()))
}

pub fn status() -> Value {
    let (last, failed) = *STATE.lock().unwrap();
    json!({"offset_us": last.map(|m| corrected(&m, pc_us())), "rtt_us": last.map(|m| m.rtt_us), "at_us": last.map(|m| m.at_us), "ok": failed.is_none()})
}

pub fn start() {
    std::thread::spawn(|| loop {
        let (last, failed) = *STATE.lock().unwrap();
        if due(last.as_ref(), failed, pc_us()) {
            match measure(&server()) {
                Ok(m) => {
                    *STATE.lock().unwrap() = (Some(m), None);
                    tracing::info!("time check: this PC is {} ms off, round trip {} ms", m.offset_us / 1000, m.rtt_us / 1000);
                }
                Err(e) => {
                    STATE.lock().unwrap().1 = Some(Instant::now());
                    if failed.is_none() {
                        tracing::warn!("time check failed: {e}");
                    }
                }
            }
        }
        std::thread::sleep(WAKE);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_790_000_000_123_456;

    fn reply(t0: i64, mode: u8, stratum: u8, t1: i64) -> Vec<u8> {
        let mut out = request(t0).to_vec();
        out[..2].copy_from_slice(&[(4 << 3) | mode, stratum]);
        out[24..32].copy_from_slice(&to_ntp(t0).to_be_bytes());
        out[32..40].copy_from_slice(&to_ntp(t1).to_be_bytes());
        out[40..48].copy_from_slice(&to_ntp(t1 + 100).to_be_bytes());
        out
    }

    #[test]
    fn a_request_asks_as_a_client_and_only_a_server_answer_to_it_is_read() {
        assert_eq!((request(T0)[0], &request(T0)[40..]), ((4 << 3) | 3, &to_ntp(T0).to_be_bytes()[..]));
        let (t1, t2) = reply_times(&reply(T0, 4, 2, T0 + 8_200_000), T0).unwrap();
        assert!((t1 - T0 - 8_200_000).abs() <= 1 && (t2 - t1 - 100).abs() <= 1);
        for bad in [reply(T0, 4, 2, 0)[..40].to_vec(), vec![7u8; PACKET], reply(T0 + 1, 4, 2, 0), reply(T0, 3, 2, 0), reply(T0, 4, 0, 0)] {
            assert_eq!(reply_times(&bad, T0), None);
        }
    }

    #[test]
    fn the_offset_is_how_far_the_server_runs_ahead_in_unix_time() {
        assert_eq!(offset_and_rtt(1_000_000, 9_210_000, 9_210_000, 1_020_000), (8_200_000, 20_000));
        assert_eq!(from_ntp(((NTP_EPOCH_S as u64 + 2) << 32) | (1 << 31)), 2_500_000);
        assert!((from_ntp(to_ntp(T0)) - T0).abs() <= 1);
    }

    #[test]
    fn a_clock_that_moved_since_the_measurement_keeps_true_time_and_asks_again_past_a_step() {
        let now = pc_us();
        let at = |moved_us: i64| Measured { offset_us: 8_200_000, rtt_us: 0, at_us: now - moved_us, at: Instant::now() };
        assert!((corrected(&at(100_000), now) - 8_100_000).abs() < 50_000 && (corrected(&at(2_000_000), now) - 6_200_000).abs() < 50_000);
        assert_eq!([due(Some(&at(100_000)), None, now), due(Some(&at(2_000_000)), None, now), due(None, None, now), due(Some(&at(0)), Some(Instant::now()), now)], [false, true, true, false]);
    }

    #[test]
    fn a_dead_server_fails_within_one_timeout() {
        let started = Instant::now();
        assert!(measure("127.0.0.1:1").is_err() && started.elapsed() < TIMEOUT + Duration::from_secs(1));
    }
}
