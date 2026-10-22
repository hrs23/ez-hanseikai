use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::room::{self, File, Set, SERIES};
use crate::{data, ffmpeg, proc, stream, Core};

const EVERY: Duration = Duration::from_secs(5);
const RUN_LIMIT: Duration = Duration::from_secs(30);
const WIDTH: u32 = 480;
const QUALITY: &str = "5";
pub const DIR: &str = "thumbs";

static GIVEN_UP: Mutex<Vec<String>> = Mutex::new(Vec::new());

enum Pick {
    Later,
    Never,
    Use(usize, String, usize),
}

pub fn path(room_dir: &Path, set: &str) -> PathBuf {
    room_dir.join(DIR).join(format!("{set}.jpg"))
}

pub fn middle(durations: &[i64]) -> Option<usize> {
    let total: i64 = durations.iter().sum();
    if durations.is_empty() || total <= 0 {
        return None;
    }
    let mut at = 0;
    for (i, d) in durations.iter().enumerate() {
        at += d;
        if at * 2 >= total {
            return Some(i);
        }
    }
    Some(durations.len() - 1)
}

fn pick(core: &Core, room: &str, set: &Set) -> Pick {
    let me = core.me();
    let mut order: Vec<usize> = (0..set.files.len()).collect();
    order.sort_by_key(|i| set.files[*i].owner != me);
    let mut waiting = false;
    for file in order {
        let f = &set.files[file];
        let (have, total) = data::held(core, room, f);
        for capture in f.captures() {
            let segs: Vec<_> = f.series(SERIES[0]).into_iter().filter(|s| s.capture == capture).collect();
            if segs.is_empty() {
                continue;
            }
            if f.end_us.is_none() || total == 0 || have < total {
                waiting = true;
                continue;
            }
            let durations: Vec<i64> = segs.iter().map(|s| s.d_us).collect();
            if let Some(i) = middle(&durations) {
                return Pick::Use(file, capture.to_string(), i);
            }
        }
    }
    if waiting { Pick::Later } else { Pick::Never }
}

fn make(core: &Core, room: &str, set: &Set, f: &File, capture: &str, index: usize) -> Result<(), String> {
    let pieces = stream::set_pieces(core, room, f, capture, SERIES[0]);
    let (init, seg) = (pieces.first().cloned(), pieces.get(index + 1).cloned());
    let (Some(init), Some(seg)) = (init, seg) else { return Err("the video is not all here".into()) };
    let both = [init, seg];
    let bytes = stream::read(&both, 0, stream::size(&both)).map_err(|e| e.to_string())?;
    let out = path(&core.room_dir(room), &set.id);
    let dir = out.parent().ok_or("no folder")?.to_path_buf();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let source = dir.join(format!("{}.tmp.mp4", set.id));
    let made = dir.join(format!("{}.tmp.jpg", set.id));
    std::fs::write(&source, bytes).map_err(|e| e.to_string())?;
    let mut command = Command::new(ffmpeg::path());
    command.args(["-v", "error", "-y", "-i"]).arg(&source).args(["-frames:v", "1", "-vf", &format!("scale={WIDTH}:-2"), "-q:v", QUALITY]).arg(&made);
    let ran = proc::run(&mut command, proc::BELOW_NORMAL, RUN_LIMIT);
    let _ = std::fs::remove_file(&source);
    let result = ran.and_then(|_| if made.exists() { std::fs::rename(&made, &out).map_err(|e| e.to_string()) } else { Err("no picture".into()) });
    let _ = std::fs::remove_file(&made);
    result
}

fn give_up(key: String) {
    GIVEN_UP.lock().unwrap().push(key);
}

fn given_up(key: &str) -> bool {
    GIVEN_UP.lock().unwrap().iter().any(|k| k == key)
}

fn make_missing(core: &Core) {
    let Some(room) = core.room_id() else { return };
    for set in room::sets(core, &room, &room::logs(core, &room)) {
        let key = format!("{room}/{}", set.id);
        if set.recording() || path(&core.room_dir(&room), &set.id).exists() || given_up(&key) {
            continue;
        }
        match pick(core, &room, &set) {
            Pick::Later => {}
            Pick::Never => give_up(key),
            Pick::Use(file, capture, index) => {
                if let Err(e) = make(core, &room, &set, &set.files[file], &capture, index) {
                    tracing::warn!("no thumbnail for {}: {e}", set.id);
                    give_up(key);
                }
            }
        }
    }
}

pub fn keeper(core: Arc<Core>) {
    loop {
        std::thread::sleep(EVERY);
        if ffmpeg::present() {
            make_missing(&core);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_middle_segment_holds_half_of_the_length() {
        assert_eq!(middle(&[]), None);
        assert_eq!(middle(&[0, 0]), None);
        assert_eq!(middle(&[4, 4, 4]), Some(1));
        assert_eq!(middle(&[4, 4, 4, 4]), Some(1));
        assert_eq!(middle(&[10, 1, 1]), Some(0));
        assert_eq!(middle(&[1, 1, 10]), Some(2));
    }
}
