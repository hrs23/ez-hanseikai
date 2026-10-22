use std::collections::BTreeMap;

use super::{keyframes, planes, Found, Job, Kind, Output, Plane, Plugin};

pub const PLUGIN: Plugin = Plugin { id: "apex-matches", name: "Apex match detection", summary: "Marks where each match ends and shows matches as chapters on the seek bar.", sample: "chapters", version: "1.1.0", kind: Kind::Analysis(run) };
pub const MINIMAP: Plugin = Plugin { id: "apex-minimap", name: "Apex minimap", summary: "Adds a tile with the minimap of the member whose app sound plays.", sample: "", version: "1.0.0", kind: Kind::View("minimap") };

const W: usize = 64;
const H: usize = 36;
const SCALE: &str = "crop='min(iw,ih*16/9)':'min(ih,iw*9/16)',scale=64:36:flags=area";
const TOP_GRAY: (usize, usize) = (3, 12);
const BOTTOM_GRAY: (usize, usize) = (22, 34);
const TOP_GRAY_MAX: f64 = 4.0;
const BOTTOM_GRAY_MAX: f64 = 3.0;
const BANNER_ROWS: (usize, usize) = (12, 22);
const BANNER_SIDE: usize = 12;
const RED_V_MIN: f64 = 133.0;
const RED_U_MAX: f64 = 126.0;
const RED_SAT_MIN: f64 = 8.0;
const MATCH_MIN_S: f64 = 150.0;

#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Frame {
    pub time: f64,
    pub result: bool,
}

struct Color {
    u: f64,
    v: f64,
    sat: f64,
}

fn color(u: &Plane, v: &Plane, x: usize, y: usize, w: usize, h: usize) -> Color {
    let sat = u.pixels(x, y, w, h).zip(v.pixels(x, y, w, h)).map(|(a, b)| (f64::from(a) - 128.0).hypot(f64::from(b) - 128.0)).sum::<f64>() / (w * h) as f64;
    Color { u: u.mean(x, y, w, h), v: v.mean(x, y, w, h), sat }
}

fn red(c: &Color) -> bool {
    c.v >= RED_V_MIN && c.u <= RED_U_MAX && c.sat >= RED_SAT_MIN
}

pub fn frame(time: f64, data: &[u8]) -> Frame {
    let [_, u, v] = planes(data, W, H);
    let rows = |(from, to): (usize, usize)| color(&u, &v, 0, from, W, to - from);
    let gray = rows(TOP_GRAY).sat <= TOP_GRAY_MAX && rows(BOTTOM_GRAY).sat <= BOTTOM_GRAY_MAX;
    let banner = (BANNER_ROWS.0..BANNER_ROWS.1).any(|y| red(&color(&u, &v, 0, y, BANNER_SIDE, 1)) && red(&color(&u, &v, W - BANNER_SIDE, y, BANNER_SIDE, 1)));
    Frame { time, result: gray && banner }
}

pub fn match_ends(frames: &[Frame]) -> Vec<f64> {
    let mut ends: Vec<f64> = Vec::new();
    let mut before = false;
    for f in frames {
        if f.result && !before && ends.last().is_none_or(|last| f.time - last > MATCH_MIN_S) {
            ends.push(f.time);
        }
        before = f.result;
    }
    ends
}

fn run(job: &Job, progress: &mut dyn FnMut(f64)) -> Result<Output, String> {
    let mut frames = Vec::new();
    keyframes(job, SCALE, W, H, |t, data| {
        frames.push(frame(t, data));
        if job.duration_s > 0.0 {
            progress(t / job.duration_s);
        }
    })?;
    let events = match_ends(&frames).into_iter().map(|t| Found { kind: "match_end".into(), at_s: Some(t), text: "Match end".into(), tags: vec!["match".into()], ..Default::default() }).collect();
    Ok(Output { events, tags: BTreeMap::from([("match".to_string(), "Match".to_string())]) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(paint: impl Fn(usize, usize) -> (u8, u8)) -> Vec<u8> {
        let mut data = vec![128u8; W * H * 3];
        for y in 0..H {
            for x in 0..W {
                let (u, v) = paint(x, y);
                data[W * H + y * W + x] = u;
                data[2 * W * H + y * W + x] = v;
            }
        }
        data
    }

    fn banner(_: usize, y: usize) -> (u8, u8) {
        if y == 13 { (120, 155) } else { (128, 128) }
    }

    #[test]
    fn a_gray_screen_with_a_red_banner_across_is_a_result() {
        assert!(frame(1.0, &picture(banner)).result);
    }

    #[test]
    fn colorful_pictures_and_one_sided_red_are_not() {
        assert!(!frame(1.0, &picture(|_, _| (128, 128))).result);
        assert!(!frame(1.0, &picture(|x, y| if y > 30 { (100, 160) } else { banner(x, y) })).result);
        assert!(!frame(1.0, &picture(|x, y| if y < 10 { (150, 150) } else { banner(x, y) })).result);
        assert!(!frame(1.0, &picture(|x, y| if x < 20 { banner(x, y) } else { (128, 128) })).result);
        assert!(!frame(1.0, &picture(|_, _| (120, 155))).result);
    }

    fn noisy(paint: impl Fn(usize, usize) -> (u8, u8)) -> impl Fn(usize, usize) -> (u8, u8) {
        move |x, y| {
            let (u, v) = paint(x, y);
            let wobble = ((x * 7 + y * 3) % 5) as i16 - 2;
            ((u as i16 + wobble) as u8, (v as i16 - wobble) as u8)
        }
    }

    #[test]
    fn the_banner_counts_anywhere_in_its_band_and_with_some_noise() {
        for row in [BANNER_ROWS.0, BANNER_ROWS.1 - 1] {
            assert!(frame(1.0, &picture(noisy(move |_, y| if y == row { (120, 155) } else { (128, 128) }))).result, "row {row}");
        }
        assert!(!frame(1.0, &picture(|_, y| if y == BANNER_ROWS.0 - 2 { (120, 155) } else { (128, 128) })).result);
    }

    #[test]
    fn wide_and_tall_pictures_are_cut_to_the_sixteen_by_nine_middle_first() {
        assert!(SCALE.starts_with("crop='min(iw,ih*16/9)':'min(ih,iw*9/16)',"));
    }

    fn frames(marks: &str, step: f64) -> Vec<Frame> {
        marks.chars().enumerate().map(|(i, c)| Frame { time: i as f64 * step, result: c == 'r' }).collect()
    }

    #[test]
    fn a_match_ends_where_its_result_first_shows() {
        assert_eq!(match_ends(&frames("...rrr...", 2.0)), vec![6.0]);
        assert!(match_ends(&frames(".........", 2.0)).is_empty());
    }

    #[test]
    fn results_closer_than_a_match_count_once() {
        let marks = format!("r{}r{}r", ".".repeat(60), ".".repeat(100));
        assert_eq!(match_ends(&frames(&marks, 2.0)), vec![0.0, 324.0]);
    }
}
