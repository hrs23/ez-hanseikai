use std::collections::BTreeMap;

use super::{keyframes, planes, Found, Job, Kind, Output, Plane, Plugin};

pub const DOWNS: Plugin = Plugin { id: "apex-downs", name: "Apex down detection", summary: "Marks where you are knocked down and where you knock down an enemy.", sample: "", version: "1.0.0", kind: Kind::Analysis(downs), options: &[] };
pub const KILLS: Plugin = Plugin { id: "apex-kills", name: "Apex kill detection", summary: "Marks where you eliminate an enemy.", sample: "", version: "1.0.0", kind: Kind::Analysis(kills), options: &[] };

const W: usize = 64;
const SCREEN_H: usize = 36;
const NOTICE_H: usize = 20;
const H: usize = SCREEN_H + NOTICE_H;
const FILTER: &str = "split[a][b];[a]scale=64:36:flags=area[s];[b]crop='min(iw,ih*16/9)':'min(ih,iw*9/16)',crop=iw*0.2:ih*0.12:iw*0.4:ih*0.55,scale=64:20:flags=area[n];[s][n]vstack";
const MIDDLE: (usize, usize) = (16, 48);
const MIDDLE_SAT_MAX: f64 = 6.0;
const SIDE: usize = 6;
const SIDE_ROWS: [(usize, usize); 2] = [(2, 8), (28, 34)];
const SIDE_V_MIN: f64 = 138.0;
const SIDE_U_MAX: f64 = 126.0;
const RED_V_MIN: u8 = 170;
const RED_U_MAX: u8 = 120;
const RED_Y_MIN: u8 = 50;
const WHITE_Y_MIN: u8 = 215;
const WHITE_CHROMA_MAX: u8 = 10;
const MARK_MIN: usize = 24;
const MARK_COLUMNS: (usize, usize) = (16, 48);
const AROUND: usize = 8;
const AROUND_MAX: usize = 8;
const DOWN_HOLD: usize = 2;
const DOWN_GAP_S: f64 = 20.0;
const NOTICE_GAP_S: f64 = 3.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Notice {
    Knock,
    Kill,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub time: f64,
    pub down: bool,
    pub notice: Option<Notice>,
}

fn sat(u: &Plane, v: &Plane, x: usize, y: usize, w: usize, h: usize) -> f64 {
    u.pixels(x, y, w, h).zip(v.pixels(x, y, w, h)).map(|(a, b)| (f64::from(a) - 128.0).hypot(f64::from(b) - 128.0)).sum::<f64>() / (w * h) as f64
}

fn red_side(u: &Plane, v: &Plane, x: usize, (from, to): (usize, usize)) -> bool {
    v.mean(x, from, SIDE, to - from) >= SIDE_V_MIN && u.mean(x, from, SIDE, to - from) <= SIDE_U_MAX
}

fn count(planes: &[Plane; 3], (x, w): (usize, usize), test: &impl Fn(u8, u8, u8) -> bool) -> usize {
    let [luma, u, v] = planes;
    let at = |p: &Plane| p.pixels(x, SCREEN_H, w, NOTICE_H).collect::<Vec<_>>();
    let (ys, us, vs) = (at(luma), at(u), at(v));
    (0..ys.len()).filter(|&i| test(ys[i], us[i], vs[i])).count()
}

fn mark(planes: &[Plane; 3], test: impl Fn(u8, u8, u8) -> bool) -> bool {
    let middle = count(planes, (MARK_COLUMNS.0, MARK_COLUMNS.1 - MARK_COLUMNS.0), &test);
    let around = count(planes, (0, AROUND), &test) + count(planes, (W - AROUND, AROUND), &test);
    middle >= MARK_MIN && around <= AROUND_MAX
}

fn red(y: u8, u: u8, v: u8) -> bool {
    v >= RED_V_MIN && u <= RED_U_MAX && y >= RED_Y_MIN
}

fn white(y: u8, u: u8, v: u8) -> bool {
    y >= WHITE_Y_MIN && u.abs_diff(128) <= WHITE_CHROMA_MAX && v.abs_diff(128) <= WHITE_CHROMA_MAX
}

pub fn frame(time: f64, data: &[u8]) -> Frame {
    let all = planes(data, W, H);
    let [_, u, v] = &all;
    let gray_middle = sat(u, v, MIDDLE.0, 0, MIDDLE.1 - MIDDLE.0, SCREEN_H) <= MIDDLE_SAT_MAX;
    let red_sides = SIDE_ROWS.iter().all(|&rows| red_side(u, v, 0, rows) && red_side(u, v, W - SIDE, rows));
    let notice = if mark(&all, red) {
        Some(Notice::Knock)
    } else if mark(&all, white) {
        Some(Notice::Kill)
    } else {
        None
    };
    Frame { time, down: gray_middle && red_sides, notice }
}

pub fn onsets(frames: &[Frame], hold: usize, gap_s: f64, on: impl Fn(&Frame) -> bool) -> Vec<f64> {
    let mut out: Vec<f64> = Vec::new();
    let mut run = 0;
    for (i, f) in frames.iter().enumerate() {
        run = if on(f) { run + 1 } else { 0 };
        let start = frames[(i + 1).saturating_sub(hold)].time;
        if run == hold && out.last().is_none_or(|last| start - last >= gap_s) {
            out.push(start);
        }
    }
    out
}

fn read(job: &Job, progress: &mut dyn FnMut(f64)) -> Result<Vec<Frame>, String> {
    let mut frames = Vec::new();
    keyframes(job, FILTER, W, H, |t, data| {
        frames.push(frame(t, data));
        if job.duration_s > 0.0 {
            progress(t / job.duration_s);
        }
    })?;
    Ok(frames)
}

fn found(kind: &str, text: &str, t: f64) -> Found {
    Found { kind: kind.into(), at_s: Some(t), text: text.into(), tags: vec![kind.into()], ..Default::default() }
}

fn downs(job: &Job, progress: &mut dyn FnMut(f64)) -> Result<Output, String> {
    let frames = read(job, progress)?;
    let downed = onsets(&frames, DOWN_HOLD, DOWN_GAP_S, |f| f.down).into_iter().map(|t| found("down", "Downed", t));
    let knocks = onsets(&frames, 1, NOTICE_GAP_S, |f| f.notice == Some(Notice::Knock)).into_iter().map(|t| found("knock", "Knock", t));
    let tags = BTreeMap::from([("down".to_string(), "Downed".to_string()), ("knock".to_string(), "Knock".to_string())]);
    Ok(Output { events: downed.chain(knocks).collect(), tags })
}

fn kills(job: &Job, progress: &mut dyn FnMut(f64)) -> Result<Output, String> {
    let frames = read(job, progress)?;
    let events = onsets(&frames, 1, NOTICE_GAP_S, |f| f.notice == Some(Notice::Kill)).into_iter().map(|t| found("kill", "Kill", t)).collect();
    Ok(Output { events, tags: BTreeMap::from([("kill".to_string(), "Kill".to_string())]) })
}

#[cfg(test)]
mod tests {
    use super::*;

    type Paint = (u8, u8, u8);

    fn picture(screen: impl Fn(usize, usize) -> Paint, notice: impl Fn(usize, usize) -> Paint) -> Vec<u8> {
        let mut data = vec![0u8; W * H * 3];
        for y in 0..H {
            for x in 0..W {
                let (l, u, v) = if y < SCREEN_H { screen(x, y) } else { notice(x, y - SCREEN_H) };
                data[y * W + x] = l;
                data[W * H + y * W + x] = u;
                data[2 * W * H + y * W + x] = v;
            }
        }
        data
    }

    const GAME: Paint = (90, 110, 150);
    const GRAY: Paint = (70, 128, 128);
    const EDGE_RED: Paint = (60, 118, 160);
    const MARK_RED: Paint = (110, 100, 200);
    const TEXT: Paint = (240, 128, 128);

    fn bleeding(x: usize, _: usize) -> Paint {
        if x < SIDE || x >= W - SIDE { EDGE_RED } else { GRAY }
    }

    fn icon(paint: Paint, n: usize) -> impl Fn(usize, usize) -> Paint {
        let w = MARK_COLUMNS.1 - MARK_COLUMNS.0;
        move |x, y| if x >= MARK_COLUMNS.0 && x < MARK_COLUMNS.1 && y * w + x - MARK_COLUMNS.0 < n { paint } else { GAME }
    }

    #[test]
    fn a_gray_screen_with_red_sides_from_top_to_bottom_is_being_down() {
        assert!(frame(1.0, &picture(bleeding, |_, _| GRAY)).down);
    }

    #[test]
    fn a_colourful_screen_red_sides_only_in_the_middle_or_the_results_banner_are_not_down() {
        assert!(!frame(1.0, &picture(|_, _| GAME, |_, _| GAME)).down);
        assert!(!frame(1.0, &picture(|x, y| if x < SIDE || x >= W - SIDE { EDGE_RED } else if y > 20 { GAME } else { GRAY }, |_, _| GRAY)).down);
        assert!(!frame(1.0, &picture(|_, y| if (12..22).contains(&y) { EDGE_RED } else { GRAY }, |_, _| GRAY)).down);
    }

    #[test]
    fn a_red_mark_under_the_crosshair_is_a_knock_and_a_white_one_a_kill() {
        assert_eq!(frame(1.0, &picture(|_, _| GAME, icon(MARK_RED, MARK_MIN))).notice, Some(Notice::Knock));
        assert_eq!(frame(1.0, &picture(|_, _| GAME, icon(TEXT, MARK_MIN))).notice, Some(Notice::Kill));
        assert_eq!(frame(1.0, &picture(|_, _| MARK_RED, |x, y| if y < 4 && (20..44).contains(&x) { TEXT } else if (28..36).contains(&x) { MARK_RED } else { GAME })).notice, Some(Notice::Knock));
    }

    #[test]
    fn a_few_bright_pixels_or_red_across_the_whole_cut_are_no_notice() {
        assert_eq!(frame(1.0, &picture(|_, _| GAME, icon(MARK_RED, MARK_MIN - 1))).notice, None);
        assert_eq!(frame(1.0, &picture(|_, _| GAME, icon(TEXT, MARK_MIN - 1))).notice, None);
        assert_eq!(frame(1.0, &picture(|_, _| GAME, |_, y| if y > 10 { MARK_RED } else { GAME })).notice, None);
        assert_eq!(frame(1.0, &picture(|_, _| GAME, |_, _| TEXT)).notice, None);
        assert_eq!(frame(1.0, &picture(|_, _| GAME, |_, _| GAME)).notice, None);
    }

    fn frames(marks: &str) -> Vec<Frame> {
        marks.chars().enumerate().map(|(i, c)| Frame { time: i as f64, down: c == 'd', notice: match c { 'k' => Some(Notice::Knock), 'x' => Some(Notice::Kill), _ => None } }).collect()
    }

    #[test]
    fn a_down_is_marked_where_it_starts_once_it_lasts_and_not_again_soon_after() {
        let list = frames(&format!("..dddd..d.dd{}dd", ".".repeat(20)));
        assert_eq!(onsets(&list, DOWN_HOLD, DOWN_GAP_S, |f| f.down), vec![2.0, 32.0]);
    }

    #[test]
    fn each_notice_counts_once_while_it_shows_and_again_after_a_gap() {
        let list = frames("..kkk..k....x.x.....x");
        assert_eq!(onsets(&list, 1, NOTICE_GAP_S, |f| f.notice == Some(Notice::Knock)), vec![2.0, 7.0]);
        assert_eq!(onsets(&list, 1, NOTICE_GAP_S, |f| f.notice == Some(Notice::Kill)), vec![12.0, 20.0]);
    }

    #[test]
    fn the_notice_is_cut_from_under_the_crosshair_after_the_sixteen_by_nine_middle_and_the_screen_is_whole() {
        assert!(FILTER.starts_with("split[a][b];[a]scale=64:36"));
        assert!(FILTER.contains("[b]crop='min(iw,ih*16/9)':'min(ih,iw*9/16)',crop="));
        assert!(FILTER.contains("scale=64:36") && FILTER.contains("scale=64:20") && FILTER.ends_with("vstack"));
    }
}
