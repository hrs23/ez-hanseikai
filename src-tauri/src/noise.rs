//! Keeps the app log readable: a line that repeats is written once per period, with a count of the ones held back.

use std::collections::HashMap;
use std::time::{Duration, Instant};

const KEYS_KEPT: usize = 256;

pub struct Repeats {
    every: Duration,
    seen: HashMap<String, (Instant, u64)>,
}

impl Repeats {
    pub fn new(every: Duration) -> Self {
        Self { every, seen: HashMap::new() }
    }

    /// Whether a line of this kind is written now: `Some(held)` with how many like it were held back since the last one, or `None` to hold it back.
    pub fn note(&mut self, key: &str, now: Instant) -> Option<u64> {
        if let Some((at, held)) = self.seen.get_mut(key) {
            if now.saturating_duration_since(*at) < self.every {
                *held += 1;
                return None;
            }
            let was = *held;
            *at = now;
            *held = 0;
            return Some(was);
        }
        if self.seen.len() >= KEYS_KEPT {
            let every = self.every;
            self.seen.retain(|_, (at, held)| *held > 0 || now.saturating_duration_since(*at) < every);
        }
        self.seen.insert(key.to_string(), (now, 0));
        Some(0)
    }

    /// The line itself, with the count held back when there is one; `None` while it is held back. Lines that differ only in numbers count as the same.
    pub fn line(&mut self, line: &str, now: Instant) -> Option<String> {
        self.note(&shape(line), now).map(|held| with_held(line, held))
    }

    /// How many lines are held back and not yet counted in a written line.
    pub fn take_held(&mut self) -> u64 {
        self.seen.values_mut().map(|(_, held)| std::mem::take(held)).sum()
    }
}

pub fn with_held(line: &str, held: u64) -> String {
    if held == 0 { line.to_string() } else { format!("{line} ({held} more like this since the last one)") }
}

/// The line with every run of digits as `#`, so that lines differing only in numbers, times or sizes match.
pub fn shape(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for c in line.chars() {
        if c.is_ascii_digit() {
            if !out.ends_with('#') {
                out.push('#');
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repeated_line_is_written_once_a_period_with_the_count_held_back() {
        let mut r = Repeats::new(Duration::from_secs(60));
        let t = Instant::now();
        assert_eq!(r.line("frame 12 late by 3 ms", t).unwrap(), "frame 12 late by 3 ms");
        assert!(r.line("frame 13 late by 4 ms", t + Duration::from_secs(1)).is_none());
        assert!(r.line("frame 14 late by 9 ms", t + Duration::from_secs(2)).is_none());
        assert_eq!(r.line("other", t + Duration::from_secs(3)).unwrap(), "other");
        assert_eq!(r.line("frame 99 late by 1 ms", t + Duration::from_secs(61)).unwrap(), "frame 99 late by 1 ms (2 more like this since the last one)");
        assert!(r.line("frame 100 late by 1 ms", t + Duration::from_secs(62)).is_none());
        assert_eq!(r.take_held(), 1);
        assert_eq!(r.take_held(), 0);
    }

    #[test]
    fn keys_are_separate() {
        let mut r = Repeats::new(Duration::from_secs(30));
        let t = Instant::now();
        assert_eq!(r.note("a", t), Some(0));
        assert_eq!(r.note("a", t + Duration::from_secs(5)), None);
        assert_eq!(r.note("b", t + Duration::from_secs(5)), Some(0));
        assert_eq!(r.note("a", t + Duration::from_secs(30)), Some(1));
    }

    #[test]
    fn digits_do_not_make_lines_different() {
        assert_eq!(shape("took 1500 ms, 3 queued"), "took # ms, # queued");
        assert_eq!(shape("no digits"), "no digits");
    }
}
