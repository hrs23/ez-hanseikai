use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};

use crate::export;
use crate::ffmpeg;
use crate::record::{self, AUDIO_KBPS, FPS};
use crate::room::{self, File, Set};
use crate::session::{self, tenth, Cue, Item, Session, View};
use crate::stream::Piece;
use crate::Core;

const TOLERANCE_S: f64 = 0.05;
const ERROR_LINES: usize = 12;
const DEFAULT_SIZE: (u32, u32) = (1280, 720);
const MATCH_TAG: &str = "match";

#[derive(Clone, Debug, PartialEq)]
pub struct Input {
    pub delay_s: f64,
    pub video: bool,
    pub audio: bool,
}

pub struct Captured {
    pub input: Input,
    pub pieces: Vec<Piece>,
}

pub fn found<T>(list: Vec<T>) -> Result<Vec<T>, String> {
    (!list.is_empty()).then_some(list).ok_or("None of this recording's files are on this PC yet.".into())
}

pub fn graph(inputs: &[Input], size: (u32, u32), total: f64) -> (String, bool) {
    let (w, h) = size;
    let mut graph = format!("color=c=black:s={w}x{h}:r={FPS}:d={total:.3}[bg0];");
    let (mut layer, mut sounds) = (0usize, Vec::new());
    for (i, input) in inputs.iter().enumerate() {
        if input.video {
            graph += &format!(
                "[{i}:v]setpts=PTS-STARTPTS+{:.3}/TB,scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1[v{i}];[bg{layer}][v{i}]overlay=eof_action=pass[bg{}];",
                input.delay_s,
                layer + 1
            );
            layer += 1;
        }
        if input.audio {
            graph += &export::delayed_sound(i, input.delay_s);
            sounds.push(format!("[s{i}]"));
        }
    }
    graph += &format!("[bg{layer}]format=yuv420p[vout]");
    if !sounds.is_empty() {
        graph += &format!(";{}[mix]", export::amix(&sounds));
    }
    (graph, !sounds.is_empty())
}

pub fn voice_graph(inputs: &[Input]) -> String {
    let sounds: Vec<String> = (0..inputs.len()).map(|i| format!("[s{i}]")).collect();
    inputs.iter().enumerate().map(|(i, input)| export::delayed_sound(i, input.delay_s)).collect::<String>() + &export::amix(&sounds) + "[mix]"
}

pub fn copyable(inputs: &[Input]) -> bool {
    matches!(inputs, [only] if only.delay_s <= TOLERANCE_S)
}

pub fn events_to_items(events: &[Value], set: &Set, owners: &[String]) -> (Vec<Item>, Vec<(String, Vec<Cue>)>) {
    let start = set.start();
    let at = |us: i64| tenth((us - start) as f64 / 1e6).max(0.0);
    let (mut items, mut ends): (Vec<Item>, Vec<f64>) = (Vec::new(), Vec::new());
    let mut speech: HashMap<String, Vec<Cue>> = HashMap::new();
    for e in events.iter().filter(|e| room::covers(set, e)) {
        let Some(local) = e["local_us"].as_i64() else { continue };
        let text = e["text"].as_str().unwrap_or_default().trim().to_string();
        let tags: Vec<String> = e["tags"].as_array().into_iter().flatten().filter_map(|t| t.as_str().map(String::from)).collect();
        let plugin = !e["plugin"].is_null();
        match (e["kind"].as_str().unwrap_or_default(), e["end_us"].as_i64()) {
            ("speech", Some(end)) => {
                if let Some(owner) = e["file"].as_str().filter(|o| owners.iter().any(|x| x == o)) {
                    speech.entry(owner.to_string()).or_default().push(Cue { from_s: at(local), to_s: at(end).max(at(local) + 0.1), text });
                }
            }
            ("drawing" | "board", _) => {}
            (_, Some(end)) if e["file"].is_null() && !text.is_empty() => items.extend(session::item(at(local), Some(at(end)), "chapter", &text, Vec::new()).ok()),
            (_, Some(_)) => {}
            (kind @ ("note" | "mark"), None) if !plugin => items.extend(session::item(at(local), None, kind, &text, tags).ok()),
            (_, None) if plugin && tags.iter().any(|t| t == MATCH_TAG) => ends.push(at(local)),
            _ => {}
        }
    }
    ends.sort_by(|a, b| a.total_cmp(b));
    let mut from = 0.0;
    for (i, end) in ends.iter().enumerate() {
        items.extend(session::item(from, Some(*end), "chapter", &format!("Match {}", i + 1), Vec::new()).ok());
        from = *end;
    }
    items.sort_by(|a, b| a.at_s.total_cmp(&b.at_s));
    let mut subtitles: Vec<(String, Vec<Cue>)> = speech.into_iter().collect();
    for (_, cues) in subtitles.iter_mut() {
        cues.sort_by(|a, b| a.from_s.total_cmp(&b.from_s));
    }
    (items, subtitles)
}

struct Output {
    name: String,
    owner: String,
    offset_ms: i64,
    start_us: i64,
    video: Vec<Captured>,
    voice: Vec<Captured>,
    size: (u32, u32),
    kbps: f64,
}

fn has_sound(piece: &Piece) -> bool {
    std::fs::read(&piece.path).is_ok_and(|bytes| bytes.windows(4).any(|w| w == b"soun"))
}

fn view_outputs(core: &Core, room_id: &str, set: &Set, f: &File, aligned: bool) -> Option<Output> {
    let (tracks, (w, h), kbps) = export::tracks(core, room_id, set, f);
    let earliest = tracks.iter().map(|t| t.delay).reduce(f64::min)?;
    let base = if aligned { 0.0 } else { earliest };
    let (video, voice): (Vec<Captured>, Vec<Captured>) = tracks
        .into_iter()
        .map(|t| Captured { input: Input { delay_s: t.delay - base, video: t.video, audio: !t.video || has_sound(&t.pieces[0]) }, pieces: t.pieces })
        .partition(|c| c.input.video);
    let size = (w.max(DEFAULT_SIZE.0) & !1, h.max(DEFAULT_SIZE.1) & !1);
    Some(Output { name: room::display_name(core, &f.owner), owner: f.owner.clone(), offset_ms: (base * 1000.0).round() as i64, start_us: set.start() + (base * 1e6) as i64, video, voice, size, kbps })
}

struct Job {
    feeds: Vec<Vec<Piece>>,
    args: Vec<String>,
    partial: PathBuf,
    path: PathBuf,
    total: f64,
}

fn encode_args(graph: &str, mapped_audio: bool, video: Option<Vec<String>>, total: f64) -> Vec<String> {
    let mut args: Vec<String> = vec!["-filter_complex".into(), graph.into()];
    if video.is_some() {
        args.extend(["-map", "[vout]"].map(String::from));
    }
    if mapped_audio {
        args.extend(["-map", "[mix]"].map(String::from));
    }
    args.extend(video.into_iter().flatten());
    if mapped_audio {
        args.extend(["-c:a".to_string(), "aac".to_string(), "-b:a".to_string(), format!("{AUDIO_KBPS}k")]);
    }
    args.extend(["-t".to_string(), format!("{total:.3}")]);
    args
}

fn run_job(core: &Arc<Core>, set: &(String, String), job: Job, work: &Path, done: usize, jobs: usize, owned: &mut bool) -> Result<(), String> {
    let mut command = export::ffmpeg_command();
    command.current_dir(work);
    let feeds = export::listen(&mut command, job.feeds)?;
    command.args(&job.args).args(["-movflags", "+faststart", "-f", "mp4"]).arg(&job.partial);
    let (stdout, stderr) = export::begin(core, set.clone(), &mut command, *owned)?;
    *owned = true;
    export::serve(core, feeds);
    let tail = export::stderr_tail(stderr, ERROR_LINES);
    export::read_progress(stdout, |s| export::emit(core, ((done as f64 + (s / job.total.max(1.0)).clamp(0.0, 1.0)) / jobs as f64) * 100.0, false, None));
    export::outcome(core, tail, &job.partial, &job.path)
}

fn job(out: &Output, video: bool, path: PathBuf, work: &Path, n: usize, total: f64) -> Job {
    let captured = if video { &out.video } else { &out.voice };
    let inputs: Vec<Input> = captured.iter().map(|c| c.input.clone()).collect();
    let args = if copyable(&inputs) {
        ["-map", "0", "-c", "copy"].map(String::from).to_vec()
    } else if video {
        let (graph, audio) = graph(&inputs, out.size, total);
        let chosen = ffmpeg::pick(&crate::default_codecs());
        let source = if out.kbps > 0.0 { out.kbps } else { record::bitrate_kbps(out.size.0, 1.0) as f64 };
        encode_args(&graph, audio, Some(ffmpeg::video_args(chosen.0, export::export_kbps(&[source], out.size, chosen.2))), total)
    } else {
        encode_args(&voice_graph(&inputs), true, None, total)
    };
    let partial = work.join(format!("part-{n}.{}", if video { "mp4" } else { "m4a" }));
    Job { feeds: captured.iter().map(|c| c.pieces.clone()).collect(), args, partial, path, total }
}

pub fn run(core: &Arc<Core>, room_id: &str, set_id: &str, dir: &Path, aligned: bool) -> Result<Value, String> {
    export::check(core)?;
    let logs = room::logs(core, room_id);
    let set = room::sets(core, room_id, &logs).into_iter().find(|s| s.id == set_id).ok_or("Unknown recording.")?;
    let set_end = set.end().ok_or("This recording is still being recorded.")?;
    if dir.is_file() || std::fs::read_dir(dir).is_ok_and(|mut d| d.next().is_some()) {
        return Err("Choose a new or empty folder.".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("The folder cannot be made: {e}"))?;
    let outputs: Vec<Output> = set.files.iter().filter_map(|f| view_outputs(core, room_id, &set, f, aligned)).collect();
    let skipped: Vec<String> = set.files.iter().filter(|f| !outputs.iter().any(|o| o.owner == f.owner)).map(|f| room::display_name(core, &f.owner)).collect();
    let outputs = found(outputs)?;
    let work = core.dir.join("export");
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let names = session::unique_names(&outputs.iter().map(|o| o.name.clone()).collect::<Vec<_>>());
    let files = session::export_names(&names);
    let set_total = (set_end - set.start()) as f64 / 1e6;
    let mut jobs = Vec::new();
    let mut views = Vec::new();
    for (out, (name, file)) in outputs.iter().zip(names.iter().zip(&files)) {
        let f = set.files.iter().find(|f| f.owner == out.owner).ok_or("Unknown recording.")?;
        let total = if aligned { set_total } else { ((f.end_us.map_or(set_end, |e| e - f.offset_us) - out.start_us) as f64 / 1e6).max(1.0) };
        let has_video = !out.video.is_empty();
        let main = format!("{file}.{}", if has_video { "mp4" } else { "m4a" });
        jobs.push(job(out, has_video, dir.join(&main), &work, jobs.len(), total));
        let voice = (has_video && !out.voice.is_empty()).then(|| format!("{file} voice.m4a"));
        if let Some(voice) = &voice {
            jobs.push(job(out, false, dir.join(voice), &work, jobs.len(), total));
        }
        views.push(View { name: name.clone(), file: main, voice, offset_ms: out.offset_ms });
    }
    let events = room::events(&logs, None);
    let owners: Vec<String> = outputs.iter().map(|o| o.owner.clone()).collect();
    let (items, subtitles) = events_to_items(&events, &set, &owners);
    let count = jobs.len();
    tracing::info!("folder export of {set_id} started: {count} files, aligned {aligned}");
    let key = (room_id.to_string(), set_id.to_string());
    let mut owned = false;
    let failure = jobs.into_iter().enumerate().try_for_each(|(n, job)| run_job(core, &key, job, &work, n, count, &mut owned)).err();
    if owned {
        export::end(core);
    }
    if let Some(e) = failure {
        let _ = std::fs::remove_dir_all(dir);
        if owned {
            export::emit(core, 0.0, true, Some(e.clone()));
        }
        return Err(e);
    }
    let title = set.files.iter().find_map(|f| f.view.as_ref().and_then(|v| v.title.clone()));
    let session = Session { id: Some(set.id.clone()), title, start_ms: Some(set.start() / 1000), views: views.clone(), items: Vec::new() };
    let write = |name: &str, text: String| std::fs::write(dir.join(name), text).map_err(|e| format!("Export failed: {e}"));
    write(session::FILE, session::session_text(&session))?;
    if items.iter().any(|i| i.kind != "chapter") {
        write(session::NOTES, session::notes_text(&items))?;
    }
    let chapters = session::chapter_cues(&items);
    if !chapters.is_empty() {
        write(session::CHAPTERS, session::vtt_text(&chapters))?;
    }
    for (owner, cues) in &subtitles {
        if let Some(view) = outputs.iter().position(|o| o.owner == *owner).map(|i| &views[i]) {
            write(&format!("{}.vtt", session::stem(&view.file)), session::vtt_text(cues))?;
        }
    }
    export::emit(core, 100.0, true, None);
    Ok(json!({"folder": dir.to_string_lossy(), "files": views.len(), "skipped": skipped}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(delay_s: f64, video: bool, audio: bool) -> Input {
        Input { delay_s, video, audio }
    }

    #[test]
    fn one_input_without_delay_is_copied_and_the_rest_is_filtered() {
        assert!(copyable(&[input(0.0, true, true)]));
        assert!(copyable(&[input(0.04, true, true)]));
        assert!(!copyable(&[input(0.5, true, true)]));
        assert!(!copyable(&[input(0.0, true, true), input(60.0, true, true)]));
        assert!(!copyable(&[]));
    }

    #[test]
    fn the_filter_places_each_capture_on_the_timeline() {
        let (g, audio) = graph(&[input(0.0, true, true), input(61.5, true, false)], (1280, 720), 120.0);
        assert!(audio);
        assert!(g.starts_with("color=c=black:s=1280x720:r=30:d=120.000[bg0];"));
        assert!(g.contains("[0:v]setpts=PTS-STARTPTS+0.000/TB"));
        assert!(g.contains("[1:v]setpts=PTS-STARTPTS+61.500/TB"));
        assert!(g.contains("[bg1][v1]overlay=eof_action=pass[bg2];"));
        assert!(g.contains("[0:a:0]asetpts=PTS-STARTPTS,adelay=delays=0:all=1[s0];"));
        assert!(!g.contains("[1:a:0]"));
        assert!(g.ends_with("[bg2]format=yuv420p[vout];[s0]amix=inputs=1:normalize=0:duration=longest[mix]"));
        let (silent, audio) = graph(&[input(2.0, true, false)], (640, 360), 10.0);
        assert!(!audio && silent.ends_with("[bg1]format=yuv420p[vout]"));
        assert_eq!(voice_graph(&[input(1.25, false, true), input(9.0, false, true)]), "[0:a:0]asetpts=PTS-STARTPTS,adelay=delays=1250:all=1[s0];[1:a:0]asetpts=PTS-STARTPTS,adelay=delays=9000:all=1[s1];[s0][s1]amix=inputs=2:normalize=0:duration=longest[mix]");
    }

    #[test]
    fn the_encode_arguments_map_only_what_exists() {
        let video = || Some(ffmpeg::video_args("libx264", 3000));
        let a = encode_args("g", true, video(), 12.0).join(" ");
        assert!(a.contains("-filter_complex g -map [vout] -map [mix] -c:v ") && a.contains("-b:v 3000k") && a.contains("-c:a aac -b:a 128k") && a.ends_with("-t 12.000"));
        let v = encode_args("g", false, video(), 12.0).join(" ");
        assert!(!v.contains("[mix]") && !v.contains("-c:a"));
        let m = encode_args("g", true, None, 12.0).join(" ");
        assert!(m.contains("-map [mix]") && !m.contains("-c:v") && !m.contains("[vout]"));
    }

    fn fake_set() -> Set {
        Set { id: "s1".into(), room: "r".into(), files: vec![File { set: "s1".into(), owner: "a".into(), start_us: 1_000_000_000, end_us: Some(1_100_000_000), offset_us: 0, segs: Vec::new(), caps: HashMap::new(), view: None }] }
    }

    fn event(kind: &str, local: i64, end: Option<i64>, text: &str, tags: Value, plugin: bool, file: Option<&str>) -> Value {
        json!({"kind": kind, "local_us": local, "end_us": end, "text": text, "tags": tags, "plugin": if plugin { json!({"id": "p"}) } else { Value::Null }, "file": file})
    }

    #[test]
    fn events_become_notes_chapters_and_subtitles_on_the_session_clock() {
        let set = fake_set();
        let s = 1_000_000_000i64;
        let events = vec![
            event("note", s + 84_500_000, None, "Late rotate #rotate", json!(["rotate"]), false, None),
            event("mark", s + 90_000_000, None, "", json!([]), false, None),
            event("chapter", s, Some(s + 30_000_000), "Warm up", json!([]), false, None),
            event("match_end", s + 60_000_000, None, "", json!(["match"]), true, None),
            event("match_end", s + 30_000_000, None, "", json!(["match"]), true, None),
            event("speech", s + 5_000_000, Some(s + 8_000_000), "Drop north", json!([]), true, Some("a")),
            event("speech", s + 6_000_000, Some(s + 7_000_000), "Elsewhere", json!([]), true, Some("zz")),
            event("drawing", s + 7_000_000, None, "", json!([]), false, None),
            event("note", s + 500_000_000, None, "Outside", json!([]), false, None),
            event("note", s + 1_000_000, None, "Plugin note", json!([]), true, None),
        ];
        let (items, subtitles) = events_to_items(&events, &set, &["a".to_string()]);
        let summary: Vec<(f64, Option<f64>, &str, &str)> = items.iter().map(|i| (i.at_s, i.to_s, i.kind.as_str(), i.text.as_str())).collect();
        assert_eq!(summary, [(0.0, Some(30.0), "chapter", "Warm up"), (0.0, Some(30.0), "chapter", "Match 1"), (30.0, Some(60.0), "chapter", "Match 2"), (84.5, None, "note", "Late rotate #rotate"), (90.0, None, "mark", "")]);
        assert_eq!(subtitles, vec![("a".to_string(), vec![Cue { from_s: 5.0, to_s: 8.0, text: "Drop north".into() }])]);
    }
}
