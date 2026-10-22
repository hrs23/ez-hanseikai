use std::path::Path;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::export::{self, clip, encode_args, Captured, Cue, Job, Note, Options, Output};
use crate::record::{AUDIO_KBPS, FPS};
use crate::room::{self, Set};
use crate::{ffmpeg, party, Core};

const FILE: &str = "debrief.mp4";
const SUBTITLES: &str = "debrief.ass";
const PIECES: &str = "pieces.txt";
const METADATA: &str = "chapters.txt";
const CAPTION_S: f64 = 4.0;
const MAX_INPUTS: usize = 16;
const LAYOUTS: [(&str, usize, usize); 4] = [("grid", 1, 9), ("side", 2, 6), ("bottom", 2, 6), ("row", 2, 4)];
const SPOTLIGHT_SPAN: usize = 3;
const ASPECT: f64 = 9.0 / 16.0;
const FONT: &str = "Segoe UI";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    pub mode: String,
    pub slots: usize,
    pub order: Vec<String>,
    pub focus: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub at_s: f64,
    pub rate: f64,
    pub layout: Layout,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub from_s: f64,
    pub to_s: f64,
    pub view: Option<View>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Burn {
    pub notes: bool,
    pub marks: bool,
    pub matches: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct Line {
    from_s: f64,
    to_s: f64,
    style: &'static str,
    pos: Option<(u32, u32)>,
    text: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

fn frame() -> f64 {
    1.0 / FPS as f64
}

fn frames(s: f64) -> i64 {
    (s * FPS as f64).round() as i64
}

fn layout_of(shared: &Value) -> Layout {
    Layout {
        mode: shared["mode"].as_str().unwrap_or("grid").to_string(),
        slots: shared["slots"].as_u64().unwrap_or(0) as usize,
        order: shared["order"].as_array().into_iter().flatten().filter_map(|k| k.as_str().map(String::from)).collect(),
        focus: shared["focus"].as_str().map(String::from),
    }
}

pub fn spans(records: &[Value], parent: &str, start_us: i64, parent_start_us: i64, parent_s: f64, total_s: f64) -> Vec<Span> {
    let mut list: Vec<(i64, &Value)> = records.iter().filter_map(|r| Some((r["t_us"].as_i64()?, &r["shared"]))).collect();
    list.sort_by_key(|(t, _)| *t);
    let at = |t_us: i64| ((t_us - start_us) as f64 / 1e6).clamp(0.0, total_s);
    let mut out: Vec<Span> = Vec::new();
    let mut push = |from_s: f64, to_s: f64, view: Option<View>| {
        if to_s <= from_s {
            return;
        }
        match out.last_mut() {
            Some(last) if last.view.is_none() && view.is_none() => last.to_s = to_s,
            _ => out.push(Span { from_s, to_s, view }),
        }
    };
    push(0.0, list.first().map_or(total_s, |(t, _)| at(*t)), None);
    for (i, (t_us, shared)) in list.iter().enumerate() {
        let (from_s, to_s) = (at(*t_us), list.get(i + 1).map_or(total_s, |(t, _)| at(*t)));
        let Some(wall_us) = shared["wall_us"].as_i64().filter(|_| shared["set"] == parent) else {
            push(from_s, to_s, None);
            continue;
        };
        let playing = shared["playing"] == true;
        let rate = shared["rate"].as_f64().filter(|r| *r > 0.0).unwrap_or(1.0);
        let stamped_us = shared["at_us"].as_i64().unwrap_or(*t_us);
        let moved = if playing { (start_us as f64 + from_s * 1e6 - stamped_us as f64) / 1e6 * rate } else { 0.0 };
        let at_s = ((wall_us - parent_start_us) as f64 / 1e6 + moved).clamp(0.0, parent_s);
        let layout = layout_of(shared);
        if !playing {
            push(from_s, to_s, Some(View { at_s, rate: 0.0, layout }));
            continue;
        }
        let ends_s = (from_s + (parent_s - at_s) / rate).min(to_s);
        push(from_s, ends_s, Some(View { at_s, rate, layout: layout.clone() }));
        push(ends_s, to_s, Some(View { at_s: parent_s, rate: 0.0, layout }));
    }
    out
}

pub fn moments(spans: &[Span], from_s: f64, to_s: f64) -> Vec<(f64, f64)> {
    let mut out: Vec<(f64, f64)> = Vec::new();
    for s in spans {
        let Some(v) = &s.view else { continue };
        let piece = if v.rate == 0.0 {
            (from_s <= v.at_s && v.at_s <= to_s).then_some((s.from_s, s.to_s))
        } else {
            let (lo, hi) = (from_s.max(v.at_s), to_s.min(v.at_s + (s.to_s - s.from_s) * v.rate));
            (lo < hi).then(|| (s.from_s + (lo - v.at_s) / v.rate, s.from_s + (hi - v.at_s) / v.rate))
        };
        let Some((a, b)) = piece else { continue };
        match out.last_mut() {
            Some(last) if a - last.1 < frame() => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

fn clock(s: f64) -> String {
    let s = s.max(0.0).floor() as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

fn captions(spans: &[Span], notes: &[Note], matches: &[Cue], name: impl Fn(&str) -> String, burn: Burn) -> (Vec<Line>, Vec<Cue>) {
    let mut lines = Vec::new();
    for n in notes.iter().filter(|n| if n.kind == "mark" { burn.marks } else { burn.notes }) {
        let text = if n.kind == "mark" { format!("Mark {}", clock(n.at_s)) } else { n.text.clone() };
        for (from_s, to_s) in moments(spans, n.at_s, n.at_s + CAPTION_S) {
            lines.push(Line { from_s, to_s, style: "Note", pos: None, text: format!("{}: {text}", name(&n.by)) });
        }
    }
    let mut chapters = Vec::new();
    for m in matches.iter().filter(|_| burn.matches) {
        for (from_s, to_s) in moments(spans, m.from_s, m.to_s) {
            lines.push(Line { from_s, to_s, style: "Match", pos: None, text: m.text.clone() });
            chapters.push(Cue { from_s, to_s, text: m.text.clone() });
        }
    }
    lines.sort_by(|a, b| a.from_s.total_cmp(&b.from_s));
    chapters.sort_by(|a, b| a.from_s.total_cmp(&b.from_s));
    (lines, chapters)
}

fn fits(mode: &str, n: usize) -> bool {
    LAYOUTS.iter().any(|(id, min, max)| *id == mode && (*min..=*max).contains(&n))
}

fn cells(n: usize, mode: &str) -> (usize, usize, Vec<(usize, usize, usize)>) {
    let mode = if fits(mode, n) { mode } else { "grid" };
    match mode {
        "row" => (n, 1, (0..n).map(|i| (i, 0, 1)).collect()),
        "side" | "bottom" => {
            let k = (n - 1).max(SPOTLIGHT_SPAN);
            let small = (0..n - 1).map(|i| if mode == "side" { (k, i, 1) } else { (i, k, 1) });
            let (cols, rows) = if mode == "side" { (k + 1, k) } else { (k, k + 1) };
            (cols, rows, std::iter::once((0, 0, k)).chain(small).collect())
        }
        _ => {
            let cols = (n.max(1) as f64).sqrt().ceil() as usize;
            let rows = n.max(1).div_ceil(cols);
            (cols, rows, (0..n).map(|i| (i % cols, i / cols, 1)).collect())
        }
    }
}

fn rects(n: usize, mode: &str, size: (u32, u32), slots: usize) -> Vec<Rect> {
    let (cols, rows, all) = cells(n.max(slots), mode);
    let (w, h) = (size.0 as f64, size.1 as f64);
    let unit = (w / cols as f64).min(h / rows as f64 / ASPECT);
    let (left, top) = ((w - cols as f64 * unit) / 2.0, (h - rows as f64 * unit * ASPECT) / 2.0);
    let even = |v: f64| (v.round() as u32) & !1;
    all.into_iter().take(n).map(|(x, y, span)| Rect { x: even(left + x as f64 * unit), y: even(top + y as f64 * unit * ASPECT), w: even(span as f64 * unit), h: even(span as f64 * unit * ASPECT) }).collect()
}

fn shown_keys(layout: &Layout, present: &[String], member: Option<&str>) -> (Vec<String>, String, usize) {
    let only = member.or(layout.focus.as_deref()).filter(|k| present.iter().any(|p| p == k));
    if let Some(k) = only {
        return (vec![k.to_string()], "grid".into(), 1);
    }
    let mut keys: Vec<String> = layout.order.iter().filter(|k| present.contains(k)).cloned().collect();
    keys.extend(present.iter().filter(|p| !layout.order.contains(p)).cloned());
    let slots = layout.slots.max(keys.len());
    let mode = if fits(&layout.mode, slots) { layout.mode.clone() } else { "grid".into() };
    (keys, mode, slots)
}

fn source(videos: &[Captured], at_s: f64, rate: f64, len_s: f64) -> Option<Captured> {
    if rate == 0.0 {
        return videos.iter().find_map(|c| {
            let p = at_s.min(c.input.end_s - frame()).max(0.0);
            clip(c, p, p + frame()).filter(|c| c.input.delay_s <= frame())
        });
    }
    videos.iter().filter_map(|c| clip(c, at_s, at_s + len_s * rate)).min_by(|a, b| a.input.delay_s.max(0.0).total_cmp(&b.input.delay_s.max(0.0)))
}

struct Tile {
    input: usize,
    delay_s: f64,
    rect: Rect,
}

fn tile_chain(t: &Tile, rate: f64, len_s: f64, label: &str) -> String {
    let i = t.input;
    let picture = if rate == 0.0 {
        format!("[{i}:v]{}trim=end_frame=1,setpts=PTS-STARTPTS,fps={FPS},tpad=stop_mode=clone:stop_duration={len_s:.3}", export::cut("", t.delay_s))
    } else {
        format!("[{i}:v]{}setpts=(PTS-STARTPTS)/{rate}+{:.3}/TB", export::cut("", t.delay_s), t.delay_s.max(0.0) / rate)
    };
    format!("{picture},scale={}:{}:force_original_aspect_ratio=decrease,setsar=1[{label}]", t.rect.w, t.rect.h)
}

fn span_graph(k: usize, len_frames: i64, rate: f64, tiles: &[Tile], size: (u32, u32)) -> String {
    let len_s = len_frames as f64 / FPS as f64;
    let mut g = format!("color=c=black:s={}x{}:r={FPS},trim=end_frame={len_frames}[c{k}x0];", size.0, size.1);
    for (j, t) in tiles.iter().enumerate() {
        let r = t.rect;
        g += &tile_chain(t, rate, len_s, &format!("t{k}x{j}"));
        g += &format!(";[c{k}x{j}][t{k}x{j}]overlay=x={}+({}-overlay_w)/2:y={}+({}-overlay_h)/2:eof_action=pass[c{k}x{}];", r.x, r.w, r.y, r.h, j + 1);
    }
    g
}

fn piece_graph(parts: &[(i64, f64, Vec<Tile>)], size: (u32, u32), offset_s: f64, subtitles: bool) -> String {
    let mut g: String = parts.iter().enumerate().map(|(k, (n, rate, tiles))| span_graph(k, *n, *rate, tiles, size)).collect();
    g += &parts.iter().enumerate().map(|(k, (_, _, tiles))| format!("[c{k}x{}]", tiles.len())).collect::<String>();
    g += &format!("concat=n={}:v=1:a=0", parts.len());
    if subtitles {
        g += &format!(",setpts=PTS+{offset_s:.6}/TB,subtitles=filename={SUBTITLES},setpts=PTS-STARTPTS");
    }
    g + ",format=yuv420p[vout]"
}

fn ass_time(s: f64) -> String {
    let cs = (s.max(0.0) * 100.0).round() as u64;
    format!("{}:{:02}:{:02}.{:02}", cs / 360_000, cs / 6000 % 60, cs / 100 % 60, cs % 100)
}

fn ass_text(text: &str) -> String {
    text.replace('\\', "\u{FF3C}").replace('{', "(").replace('}', ")").replace("\r\n", "\\N").replace('\n', "\\N")
}

fn ass(lines: &[Line], size: (u32, u32)) -> String {
    let (w, h) = size;
    let (big, small, margin) = (h / 22, h / 40, h / 40);
    let mut out = format!(
        "[Script Info]\nScriptType: v4.00+\nPlayResX: {w}\nPlayResY: {h}\nWrapStyle: 0\nScaledBorderAndShadow: yes\n\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
         Style: Note,{FONT},{big},&H00FFFFFF,&H00FFFFFF,&H00000000,&H80000000,0,0,0,0,100,100,0,0,1,3,0,2,{margin},{margin},{margin},1\n\
         Style: Match,{FONT},{big},&H00FFFFFF,&H00FFFFFF,&H00000000,&H80000000,1,0,0,0,100,100,0,0,1,3,0,7,{margin},{margin},{margin},1\n\
         Style: Name,{FONT},{small},&H00FFFFFF,&H00FFFFFF,&H00000000,&H80000000,0,0,0,0,100,100,0,0,1,2,0,1,0,0,0,1\n\n\
         [Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n"
    );
    for l in lines {
        let pos = l.pos.map_or(String::new(), |(x, y)| format!("{{\\pos({x},{y})}}"));
        out += &format!("Dialogue: 0,{},{},{},,0,0,0,,{pos}{}\n", ass_time(l.from_s), ass_time(l.to_s), l.style, ass_text(&l.text));
    }
    out
}

fn metadata(chapters: &[Cue]) -> String {
    let mut out = String::from(";FFMETADATA1\n");
    for c in chapters {
        out += &format!("[CHAPTER]\nTIMEBASE=1/1000\nSTART={}\nEND={}\ntitle={}\n", (c.from_s * 1000.0).round() as u64, (c.to_s * 1000.0).round() as u64, c.text.replace(['=', ';', '#', '\\', '\n'], " "));
    }
    out
}

struct Piece {
    first: i64,
    parts: Vec<(i64, f64, Vec<Tile>)>,
    feeds: Vec<Captured>,
}

fn plan(spans: &[Span], players: &[Output], member: Option<&str>, size: (u32, u32), lines: &mut Vec<Line>) -> Vec<Piece> {
    let present: Vec<String> = players.iter().map(|p| p.owner.clone()).collect();
    let mut pieces: Vec<Piece> = Vec::new();
    for s in spans {
        let n = frames(s.to_s) - frames(s.from_s);
        if n <= 0 {
            continue;
        }
        let len_s = n as f64 / FPS as f64;
        let mut picked: Vec<(Captured, Rect)> = Vec::new();
        let rate = s.view.as_ref().map_or(0.0, |v| v.rate);
        if let Some(v) = &s.view {
            let (keys, mode, slots) = shown_keys(&v.layout, &present, member);
            for (key, rect) in keys.iter().zip(rects(keys.len(), &mode, size, slots)) {
                let Some(out) = players.iter().find(|p| &p.owner == key) else { continue };
                let margin = rect.h / 40;
                lines.push(Line { from_s: frames(s.from_s) as f64 / FPS as f64, to_s: frames(s.to_s) as f64 / FPS as f64, style: "Name", pos: Some((rect.x + margin, rect.y + rect.h - margin)), text: out.name.clone() });
                if let Some(c) = source(&out.video, v.at_s, rate, len_s) {
                    picked.push((c, rect));
                }
            }
        }
        if pieces.last().is_none_or(|p| p.feeds.len() + picked.len() > MAX_INPUTS) {
            pieces.push(Piece { first: frames(s.from_s), parts: Vec::new(), feeds: Vec::new() });
        }
        let piece = pieces.last_mut().unwrap();
        let tiles = picked.iter().enumerate().map(|(j, (c, rect))| Tile { input: piece.feeds.len() + j, delay_s: c.input.delay_s, rect: *rect }).collect();
        piece.feeds.extend(picked.into_iter().map(|(c, _)| c));
        piece.parts.push((n, rate, tiles));
    }
    pieces
}

fn final_args(delays: &[f64], chapters: bool, total: f64) -> Vec<String> {
    let voices = delays.len();
    let mut args: Vec<String> = ["-f", "concat", "-safe", "0", "-i", PIECES].map(String::from).to_vec();
    if chapters {
        args.extend(["-f", "ffmetadata", "-i", METADATA].map(String::from));
    }
    let labels: Vec<String> = (0..voices).map(|i| format!("[s{i}]")).collect();
    let graph = delays.iter().enumerate().map(|(i, d)| export::delayed_sound(i, *d)).collect::<String>() + &export::amix(&labels) + "[mix]";
    args.extend(["-filter_complex".to_string(), graph, "-map".into(), format!("{voices}:v:0"), "-map".into(), "[mix]".into()]);
    if chapters {
        args.extend(["-map_chapters".to_string(), format!("{}", voices + 1)]);
    }
    args.extend(["-c:v", "copy", "-c:a", "aac", "-b:a"].map(String::from));
    args.extend([format!("{AUDIO_KBPS}k"), "-t".into(), format!("{total:.3}")]);
    args
}

pub fn video(core: &Arc<Core>, room_id: &str, logs: &[room::Log], set: &Set, total: f64, dir: &Path, opts: &Options) -> Result<Value, String> {
    let parent_id = room::debrief_of(&set.id, logs).ok_or("This recording is not a debrief.")?;
    let parent = room::sets(core, room_id, logs).into_iter().find(|s| s.id == parent_id).ok_or("Unknown recording.")?;
    let parent_s = (parent.end_shown().ok_or("This recording is still being recorded.")? - parent.start()) as f64 / 1e6;
    if opts.member.as_ref().is_some_and(|m| !parent.files.iter().any(|f| &f.owner == m)) {
        return Err("That member is not in this recording.".into());
    }
    let players: Vec<Output> = parent.files.iter().filter_map(|f| export::view_outputs(core, room_id, &parent, f)).filter(|o| !o.video.is_empty()).collect();
    let skipped: Vec<String> = parent.files.iter().filter(|f| !players.iter().any(|o| o.owner == f.owner)).map(|f| room::display_name(core, &f.owner)).collect();
    let voices: Vec<Captured> = set.files.iter().filter_map(|f| export::view_outputs(core, room_id, set, f)).flat_map(|o| o.voice).collect();
    if players.is_empty() || voices.is_empty() {
        return Err("None of this recording's files are on this PC yet.".into());
    }
    let records = party::records(logs).remove(&set.id).unwrap_or_default();
    let spans = spans(&records, &parent.id, set.start(), parent.start(), parent_s, total);
    let items = export::events_to_items(&room::events(logs, None), &parent);
    let (mut lines, chapters) = captions(&spans, &items.notes, &items.matches, |id| room::display_name(core, id), opts.burn);
    let chosen = ffmpeg::pick(&crate::default_codecs());
    let largest = players.iter().fold((0, 0), |a, p| (a.0.max(p.size.0), a.1.max(p.size.1)));
    let source = players.iter().map(|p| p.kbps).fold(0.0, f64::max);
    let (size, kbps) = export::picture(opts.quality, largest, if source > 0.0 { source } else { crate::record::bitrate_kbps(largest.0, 1.0) as f64 }, chosen.2);
    let pieces = plan(&spans, &players, opts.member.as_deref(), size, &mut lines);
    std::fs::create_dir_all(dir).map_err(|e| format!("The folder cannot be made: {e}"))?;
    let work = core.dir.join("export");
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let write = |name: &str, text: String| std::fs::write(work.join(name), text).map_err(|e| format!("Export failed: {e}"));
    let subtitles = !lines.is_empty();
    if subtitles {
        write(SUBTITLES, ass(&lines, size))?;
    }
    if !chapters.is_empty() {
        write(METADATA, metadata(&chapters))?;
    }
    write(PIECES, (0..pieces.len()).map(|n| format!("file 'piece-{n}.mp4'\n")).collect())?;
    let mut jobs: Vec<Job> = pieces
        .iter()
        .enumerate()
        .map(|(n, p)| {
            let len: f64 = p.parts.iter().map(|(f, _, _)| *f as f64 / FPS as f64).sum();
            let graph = piece_graph(&p.parts, size, p.first as f64 / FPS as f64, subtitles);
            let path = work.join(format!("piece-{n}.mp4"));
            Job { feeds: p.feeds.iter().map(|c| c.pieces.clone()).collect(), args: encode_args(&graph, false, Some(ffmpeg::video_args(chosen.0, kbps)), len), partial: work.join(format!("piece-{n}.part.mp4")), path, total: len }
        })
        .collect();
    jobs.push(Job { feeds: voices.iter().map(|c| c.pieces.clone()).collect(), args: final_args(&voices.iter().map(|c| c.input.delay_s).collect::<Vec<_>>(), !chapters.is_empty(), total), partial: work.join("debrief.part.mp4"), path: dir.join(FILE), total });
    tracing::info!("debrief video export of {} started: {} pieces", set.id, pieces.len());
    let result = export::run_all(core, (room_id.to_string(), set.id.clone()), jobs, &work, dir);
    for name in [SUBTITLES, METADATA, PIECES].into_iter().map(String::from).chain((0..pieces.len()).map(|n| format!("piece-{n}.mp4"))) {
        let _ = std::fs::remove_file(work.join(name));
    }
    result?;
    export::emit_done(core, dir);
    Ok(json!({"folder": dir.to_string_lossy(), "files": 1, "skipped": skipped}))
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: i64 = 1_000_000_000;
    const P: i64 = 500_000_000;

    fn rec(t_s: f64, set: &str, pos_s: f64, playing: bool, rate: f64) -> Value {
        let t_us = S + (t_s * 1e6) as i64;
        json!({"t_us": t_us, "by": "a", "shared": {"set": set, "wall_us": P + (pos_s * 1e6) as i64, "at_us": t_us, "playing": playing, "rate": rate, "mode": "grid", "slots": 0, "order": ["b", "a"], "focus": null}})
    }

    fn brief(spans: &[Span]) -> Vec<(f64, f64, Option<(f64, f64)>)> {
        spans.iter().map(|s| (s.from_s, s.to_s, s.view.as_ref().map(|v| (v.at_s, v.rate)))).collect()
    }

    #[test]
    fn no_party_records_leave_the_whole_debrief_black() {
        assert_eq!(brief(&spans(&[], "rv", S, P, 600.0, 90.0)), [(0.0, 90.0, None)]);
        assert!(moments(&spans(&[], "rv", S, P, 600.0, 90.0), 0.0, 1e9).is_empty());
    }

    #[test]
    fn plays_pauses_rates_and_seeks_become_spans_of_the_debrief() {
        let records = [rec(10.0, "rv", 100.0, true, 1.0), rec(20.0, "rv", 110.0, false, 1.0), rec(30.0, "rv", 50.0, true, 2.0), rec(40.0, "rv", 70.0, true, 0.5)];
        let s = spans(&records, "rv", S, P, 600.0, 60.0);
        assert_eq!(brief(&s), [(0.0, 10.0, None), (10.0, 20.0, Some((100.0, 1.0))), (20.0, 30.0, Some((110.0, 0.0))), (30.0, 40.0, Some((50.0, 2.0))), (40.0, 60.0, Some((70.0, 0.5)))]);
        assert_eq!(s[1].view.as_ref().unwrap().layout.order, ["b", "a"]);
    }

    #[test]
    fn records_are_sorted_and_the_later_of_two_at_one_moment_wins() {
        let records = [rec(20.0, "rv", 5.0, false, 1.0), rec(10.0, "rv", 1.0, true, 1.0), rec(20.0, "rv", 9.0, true, 1.0)];
        assert_eq!(brief(&spans(&records, "rv", S, P, 600.0, 30.0)), [(0.0, 10.0, None), (10.0, 20.0, Some((1.0, 1.0))), (20.0, 30.0, Some((9.0, 1.0)))]);
    }

    #[test]
    fn another_set_or_a_record_without_position_is_a_gap() {
        let mut bare = rec(20.0, "rv", 0.0, true, 1.0);
        bare["shared"]["wall_us"] = Value::Null;
        let records = [rec(0.0, "rv", 0.0, true, 1.0), rec(10.0, "other", 0.0, true, 1.0), bare, rec(30.0, "rv", 40.0, false, 1.0)];
        assert_eq!(brief(&spans(&records, "rv", S, P, 600.0, 40.0)), [(0.0, 10.0, Some((0.0, 1.0))), (10.0, 30.0, None), (30.0, 40.0, Some((40.0, 0.0)))]);
    }

    #[test]
    fn a_stale_stamp_moves_the_position_and_records_outside_the_debrief_are_clamped() {
        let mut early = rec(-5.0, "rv", 100.0, true, 2.0);
        early["shared"]["at_us"] = json!(S - 8_000_000);
        let after = rec(70.0, "rv", 0.0, false, 1.0);
        assert_eq!(brief(&spans(&[early, after], "rv", S, P, 600.0, 60.0)), [(0.0, 60.0, Some((116.0, 2.0)))]);
    }

    #[test]
    fn playing_past_the_end_of_the_recording_freezes_on_its_last_frame() {
        let s = spans(&[rec(0.0, "rv", 590.0, true, 2.0)], "rv", S, P, 600.0, 20.0);
        assert_eq!(brief(&s), [(0.0, 5.0, Some((590.0, 2.0))), (5.0, 20.0, Some((600.0, 0.0)))]);
        assert_eq!(brief(&spans(&[rec(0.0, "rv", 900.0, false, 0.0)], "rv", S, P, 600.0, 5.0)), [(0.0, 5.0, Some((600.0, 0.0)))]);
    }

    #[test]
    fn a_moment_is_shown_each_time_the_party_passes_it_or_pauses_on_it() {
        let records = [rec(0.0, "rv", 100.0, true, 1.0), rec(10.0, "rv", 95.0, true, 2.0), rec(20.0, "rv", 102.0, false, 1.0), rec(25.0, "rv", 300.0, true, 1.0)];
        let s = spans(&records, "rv", S, P, 600.0, 30.0);
        assert_eq!(moments(&s, 102.0, 106.0), [(2.0, 6.0), (13.5, 15.5), (20.0, 25.0)]);
        assert_eq!(moments(&s, 300.0, 300.0), Vec::<(f64, f64)>::new());
        assert_eq!(moments(&s, 299.0, 301.0), [(25.0, 26.0)]);
        assert!(moments(&s, 0.0, 50.0).is_empty());
    }

    #[test]
    fn touching_pieces_of_one_moment_merge() {
        let records = [rec(0.0, "rv", 10.0, true, 1.0), rec(5.0, "rv", 15.0, true, 2.0)];
        assert_eq!(moments(&spans(&records, "rv", S, P, 600.0, 10.0), 0.0, 100.0), [(0.0, 10.0)]);
    }

    fn note(at_s: f64, kind: &str, text: &str) -> Note {
        Note { at_s, kind: kind.into(), text: text.into(), tags: Vec::new(), by: "a".into() }
    }

    #[test]
    fn notes_marks_and_matches_become_captions_and_chapters_on_the_debrief_clock() {
        let s = spans(&[rec(0.0, "rv", 100.0, true, 1.0)], "rv", S, P, 600.0, 60.0);
        let notes = [note(110.0, "note", "Rotate {now}"), note(120.0, "mark", ""), note(500.0, "note", "Never shown")];
        let matches = [Cue { from_s: 0.0, to_s: 130.0, text: "Match 1".into() }, Cue { from_s: 130.0, to_s: 400.0, text: "Match 2".into() }];
        let all = Burn { notes: true, marks: true, matches: true };
        let (lines, chapters) = captions(&s, &notes, &matches, |id| format!("Ann {id}"), all);
        let got: Vec<(f64, f64, &str, &str)> = lines.iter().map(|l| (l.from_s, l.to_s, l.style, l.text.as_str())).collect();
        assert_eq!(got, [(0.0, 30.0, "Match", "Match 1"), (10.0, 14.0, "Note", "Ann a: Rotate {now}"), (20.0, 24.0, "Note", "Ann a: Mark 2:00"), (30.0, 60.0, "Match", "Match 2")]);
        assert_eq!(chapters.iter().map(|c| (c.from_s, c.to_s, c.text.as_str())).collect::<Vec<_>>(), [(0.0, 30.0, "Match 1"), (30.0, 60.0, "Match 2")]);
        let (lines, chapters) = captions(&s, &notes, &matches, |id| id.to_string(), Burn { notes: false, marks: true, matches: false });
        assert_eq!((lines.len(), lines[0].text.as_str(), chapters.len()), (1, "a: Mark 2:00", 0));
        assert!(ass(&lines, (1920, 1080)).contains("Dialogue: 0,0:00:20.00,0:00:24.00,Note,,0,0,0,,a: Mark 2:00\n"));
        assert_eq!(ass_text("a\\b {x}\nc"), "a＼b (x)\\Nc");
        assert_eq!(metadata(&chapters), ";FFMETADATA1\n");
        assert_eq!(metadata(&[Cue { from_s: 1.5, to_s: 30.0, text: "Match 1".into() }]), ";FFMETADATA1\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=1500\nEND=30000\ntitle=Match 1\n");
    }

    #[test]
    fn tiles_follow_the_party_layout_like_the_player() {
        let present = ["a".to_string(), "b".to_string(), "c".to_string()];
        let grid = Layout { mode: "grid".into(), slots: 0, order: vec!["c".into()], focus: None };
        assert_eq!(shown_keys(&grid, &present, None), (vec!["c".into(), "a".into(), "b".into()], "grid".into(), 3));
        assert_eq!(shown_keys(&Layout { focus: Some("b".into()), ..grid.clone() }, &present, None).0, ["b"]);
        assert_eq!(shown_keys(&grid, &present, Some("a")).0, ["a"]);
        assert_eq!(shown_keys(&Layout { mode: "row".into(), slots: 6, ..grid.clone() }, &present, None).1, "grid");
        assert_eq!(rects(1, "grid", (1920, 1080), 1), [Rect { x: 0, y: 0, w: 1920, h: 1080 }]);
        assert_eq!(rects(2, "grid", (1920, 1080), 2), [Rect { x: 0, y: 270, w: 960, h: 540 }, Rect { x: 960, y: 270, w: 960, h: 540 }]);
        assert_eq!(rects(3, "grid", (1920, 1080), 3), [Rect { x: 0, y: 0, w: 960, h: 540 }, Rect { x: 960, y: 0, w: 960, h: 540 }, Rect { x: 0, y: 540, w: 960, h: 540 }]);
        let side = rects(2, "side", (1920, 1080), 2);
        assert_eq!((side[0].w, side[1].w, side[1].x), (1440, 480, 1440));
    }

    #[test]
    fn the_graph_cuts_freezes_and_tiles_each_span() {
        let rect = Rect { x: 960, y: 270, w: 960, h: 540 };
        let parts = vec![(300, 2.0, vec![Tile { input: 0, delay_s: -3.5, rect }]), (90, 0.0, vec![Tile { input: 1, delay_s: -1.0, rect }]), (30, 0.0, Vec::new())];
        let g = piece_graph(&parts, (1920, 1080), 12.5, true);
        assert!(g.starts_with("color=c=black:s=1920x1080:r=30,trim=end_frame=300[c0x0];[0:v]setpts=PTS-STARTPTS,trim=start=3.500,setpts=(PTS-STARTPTS)/2+0.000/TB,scale=960:540:force_original_aspect_ratio=decrease,setsar=1[t0x0];[c0x0][t0x0]overlay=x=960+(960-overlay_w)/2:y=270+(540-overlay_h)/2:eof_action=pass[c0x1];"));
        assert!(g.contains("[1:v]setpts=PTS-STARTPTS,trim=start=1.000,trim=end_frame=1,setpts=PTS-STARTPTS,fps=30,tpad=stop_mode=clone:stop_duration=3.000,scale="));
        assert!(g.contains("color=c=black:s=1920x1080:r=30,trim=end_frame=30[c2x0];[c0x1][c1x1][c2x0]concat=n=3:v=1:a=0,setpts=PTS+12.500000/TB,subtitles=filename=debrief.ass,setpts=PTS-STARTPTS,format=yuv420p[vout]"));
        assert!(piece_graph(&parts[2..], (1280, 720), 0.0, false).ends_with("[c0x0]concat=n=1:v=1:a=0,format=yuv420p[vout]"));
        let late = tile_chain(&Tile { input: 2, delay_s: 4.0, rect }, 0.5, 10.0, "x");
        assert!(late.starts_with("[2:v]setpts=(PTS-STARTPTS)/0.5+8.000/TB,scale="));
    }

    #[test]
    fn the_last_step_joins_the_pieces_with_every_voice_and_the_chapters() {
        let a = final_args(&[0.0, 2.5], true, 61.0).join(" ");
        assert!(a.starts_with("-f concat -safe 0 -i pieces.txt -f ffmetadata -i chapters.txt -filter_complex [0:a:0]asetpts=PTS-STARTPTS,adelay=delays=0:all=1[s0];[1:a:0]asetpts=PTS-STARTPTS,adelay=delays=2500:all=1[s1];"));
        assert!(a.contains("[s0][s1]amix=inputs=2:normalize=0:duration=longest,apad[mix] -map 2:v:0 -map [mix] -map_chapters 3 -c:v copy -c:a aac") && a.ends_with("-t 61.000"));
        let b = final_args(&[0.0], false, 5.0).join(" ");
        assert!(!b.contains("ffmetadata") && !b.contains("-map_chapters") && b.contains("-map 1:v:0"));
    }

    fn captured(times: Vec<(f64, f64)>) -> Captured {
        let piece = |n: &str| crate::stream::Piece { path: std::path::PathBuf::from(n), len: 1 };
        let pieces = std::iter::once(piece("init")).chain(times.iter().map(|_| piece("s"))).collect();
        Captured { input: export::Input { delay_s: times[0].0, end_s: times.last().unwrap().1, video: true, audio: false }, pieces, times }
    }

    #[test]
    fn each_span_takes_only_the_segments_it_shows() {
        let c = captured(vec![(0.0, 4.0), (4.0, 8.0), (8.0, 12.0)]);
        let play = source(std::slice::from_ref(&c), 5.0, 2.0, 2.0).unwrap();
        assert_eq!((play.pieces.len(), play.input.delay_s), (3, -1.0));
        let frozen = source(std::slice::from_ref(&c), 12.0, 0.0, 9.0).unwrap();
        assert_eq!(frozen.pieces.len(), 2);
        assert!(frozen.input.delay_s < 0.0);
        let late = captured(vec![(20.0, 24.0)]);
        assert!(source(std::slice::from_ref(&late), 10.0, 0.0, 1.0).is_none());
        assert_eq!(source(&[late], 18.0, 1.0, 4.0).unwrap().input.delay_s, 2.0);
    }

    #[test]
    fn pieces_hold_at_most_a_few_inputs_and_start_on_the_frame_grid() {
        let out = |owner: &str| Output { name: owner.to_uppercase(), owner: owner.into(), video: vec![captured(vec![(0.0, 600.0)])], voice: Vec::new(), size: (1920, 1080), kbps: 6000.0 };
        let players = [out("a"), out("b")];
        let records: Vec<Value> = (0..20).map(|i| rec(i as f64, "rv", 10.0 * i as f64, i % 2 == 0, 1.0)).collect();
        let s = spans(&records, "rv", S, P, 600.0, 20.0);
        let mut lines = Vec::new();
        let pieces = plan(&s, &players, None, (1920, 1080), &mut lines);
        assert!(pieces.iter().all(|p| p.feeds.len() <= MAX_INPUTS));
        assert_eq!(pieces.iter().map(|p| p.parts.len()).sum::<usize>(), 20);
        assert_eq!(pieces.iter().map(|p| p.parts.iter().map(|x| x.0).sum::<i64>()).sum::<i64>(), 600);
        assert_eq!(pieces[1].first, 240);
        assert_eq!((lines.len(), lines[0].text.as_str(), lines[1].text.as_str(), lines[0].pos), (40, "B", "A", Some((13, 797))));
        let one = plan(&s, &players, Some("a"), (1920, 1080), &mut Vec::new());
        assert_eq!(one.iter().map(|p| p.feeds.len()).sum::<usize>(), 20);
    }
}
