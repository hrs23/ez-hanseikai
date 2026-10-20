use serde_json::{json, Value};

use crate::{clean_name, clean_text};

pub const FILE: &str = "session.json";
pub const NOTES: &str = "notes.csv";
pub const CHAPTERS: &str = "chapters.vtt";
pub const VIEWS_MAX: usize = 6;
pub const TITLE_MAX: usize = 80;
pub const TEXT_MAX: usize = 500;
pub const TAGS_MAX: usize = 16;
pub const OFFSET_MAX_MS: i64 = 24 * 3600 * 1000;
pub const AT_MAX_S: f64 = 7.0 * 24.0 * 3600.0;
pub const VIDEO_EXTENSIONS: [&str; 8] = ["mp4", "mkv", "mov", "webm", "avi", "m4v", "ts", "flv"];
pub const AUDIO_EXTENSIONS: [&str; 7] = ["m4a", "mp3", "wav", "aac", "ogg", "opus", "flac"];

#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub name: String,
    pub file: String,
    pub voice: Option<String>,
    pub offset_ms: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub at_s: f64,
    pub to_s: Option<f64>,
    pub kind: String,
    pub text: String,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Session {
    pub id: Option<String>,
    pub title: Option<String>,
    pub start_ms: Option<i64>,
    pub views: Vec<View>,
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    pub from_s: f64,
    pub to_s: f64,
    pub text: String,
}

pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

pub fn civil_seconds(y: i64, m: i64, d: i64, h: i64, min: i64, s: i64) -> Option<i64> {
    let valid = (1..=12).contains(&m) && (1..=31).contains(&d) && (0..=23).contains(&h) && (0..=59).contains(&min) && (0..=59).contains(&s);
    valid.then(|| ((days_from_civil(y, m, d) * 24 + h) * 60 + min) * 60 + s)
}

pub fn parse_iso(text: &str) -> Option<i64> {
    let text = text.trim().strip_suffix('Z')?;
    let (date, time) = text.split_once('T')?;
    let mut d = date.split('-');
    let (y, m, day): (i64, i64, i64) = (d.next()?.parse().ok()?, d.next()?.parse().ok()?, d.next()?.parse().ok()?);
    let (clock, fraction) = time.split_once('.').unwrap_or((time, ""));
    let mut t = clock.split(':');
    let (h, min, s): (i64, i64, i64) = (t.next()?.parse().ok()?, t.next()?.parse().ok()?, t.next().unwrap_or("0").parse().ok()?);
    if d.next().is_some() || t.next().is_some() || !(1970..=9999).contains(&y) || fraction.len() > 9 || !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let millis: i64 = format!("{fraction:0<3}")[..3].parse().ok()?;
    Some(civil_seconds(y, m, day, h, min, s)? * 1000 + millis)
}

pub fn iso(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let z = secs.div_euclid(86400) + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let rest = secs.rem_euclid(86400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z", rest / 3600, rest % 3600 / 60, rest % 60, ms.rem_euclid(1000))
}

pub fn extension(name: &str) -> String {
    name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default()
}

pub fn stem(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or(name);
    last.rsplit_once('.').map_or(last, |(s, _)| s).to_string()
}

pub fn is_video(name: &str) -> bool {
    VIDEO_EXTENSIONS.contains(&extension(name).as_str())
}

pub fn is_media(name: &str) -> bool {
    is_video(name) || AUDIO_EXTENSIONS.contains(&extension(name).as_str())
}

pub fn tenth(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

pub fn safe_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with(['/', '\\'])
        && !path.contains(':')
        && !path.chars().any(|c| c.is_control())
        && path.split(['/', '\\']).all(|part| !part.is_empty() && part != "." && part != "..")
}

pub fn file_name(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect();
    let trimmed = cleaned.trim().trim_end_matches('.').trim();
    if trimmed.is_empty() { "view".into() } else { trimmed.chars().take(60).collect() }
}

pub fn unique_names(names: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for name in names {
        let base = if name.is_empty() { "View".to_string() } else { name.clone() };
        let mut candidate = base.clone();
        let mut n = 2;
        while out.iter().any(|o| o.to_lowercase() == candidate.to_lowercase()) {
            let suffix = format!(" {n}");
            candidate = format!("{}{suffix}", base.chars().take(crate::NAME_MAX - suffix.len()).collect::<String>());
            n += 1;
        }
        out.push(candidate);
    }
    out
}

pub fn export_names(names: &[String]) -> Vec<String> {
    let stems: Vec<String> = names.iter().map(|n| file_name(n)).map(|f| if ["session", "notes", "chapters"].contains(&f.to_lowercase().as_str()) { format!("{f} view") } else { f }).collect();
    unique_names(&stems)
}

fn number(value: &Value, what: &str) -> Result<f64, String> {
    value.as_f64().filter(|v| v.is_finite()).ok_or_else(|| format!("{what} must be a number."))
}

fn list<'a>(value: &'a Value, message: &str) -> Result<&'a [Value], String> {
    match value {
        Value::Null => Ok(&[]),
        Value::Array(list) => Ok(list),
        _ => Err(message.into()),
    }
}

fn views_fit(n: usize) -> Result<(), String> {
    if n > VIEWS_MAX { Err(format!("A recording has at most {VIEWS_MAX} views.")) } else { Ok(()) }
}

fn tag_list(value: &Value) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for tag in list(value, "tags must be a list of words.")? {
        let tag: String = tag.as_str().ok_or("tags must be a list of words.")?.trim_start_matches('#').chars().filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-').collect();
        if !tag.is_empty() && !out.iter().any(|t| t.to_lowercase() == tag.to_lowercase()) {
            out.push(tag);
        }
    }
    if out.len() > TAGS_MAX {
        return Err("Too many tags.".into());
    }
    Ok(out)
}

pub fn item(at_s: f64, to_s: Option<f64>, kind: &str, text: &str, tags: Vec<String>) -> Result<Item, String> {
    if !(0.0..=AT_MAX_S).contains(&at_s) {
        return Err("A time must be seconds from the start of the recording.".into());
    }
    let text = crate::clean_note(text, TEXT_MAX).trim().to_string();
    match kind {
        "note" if text.is_empty() => Err("A note needs text.".into()),
        "note" | "mark" if to_s.is_some() => Err("Only a chapter has to_s.".into()),
        "chapter" if text.is_empty() || to_s.is_none_or(|to| to <= at_s || to > AT_MAX_S) => Err("A chapter needs text and a to_s after at_s.".into()),
        "note" | "mark" | "chapter" => Ok(Item { at_s: tenth(at_s), to_s: to_s.map(tenth), kind: kind.into(), text, tags }),
        _ => Err("kind must be note, mark or chapter.".into()),
    }
}

fn parse_item(value: &Value) -> Result<Item, String> {
    let at_s = number(&value["at_s"], "at_s")?;
    let to_s = match &value["to_s"] {
        Value::Null => None,
        v => Some(number(v, "to_s")?),
    };
    let kind = match &value["kind"] {
        Value::Null => if to_s.is_some() { "chapter" } else if value["text"].is_null() { "mark" } else { "note" },
        Value::String(k) => k.as_str(),
        _ => return Err("kind must be note, mark or chapter.".into()),
    };
    let text = match &value["text"] {
        Value::Null => "",
        Value::String(t) => t.as_str(),
        _ => return Err("text must be a string.".into()),
    };
    item(at_s, to_s, kind, text, tag_list(&value["tags"])?)
}

fn parse_view(value: &Value) -> Result<View, String> {
    let text = |key: &str| match &value[key] {
        Value::Null => Ok(None),
        Value::String(s) if !s.trim().is_empty() => Ok(Some(s.trim().to_string())),
        _ => Err(format!("{key} must be a text.")),
    };
    let (name, file, voice) = (text("name")?, text("file")?, text("voice")?);
    let name = match (&name, &file) {
        (Some(name), _) => clean_name(name),
        (None, Some(file)) => clean_name(&stem(file)),
        (None, None) => return Err("A view needs a file or a name.".into()),
    };
    let file = file.unwrap_or_else(|| format!("{name}.mp4"));
    for path in std::iter::once(&file).chain(voice.iter()) {
        if !safe_relative(path) {
            return Err(format!("{path} must be a file inside the folder."));
        }
    }
    let offset_ms = match &value["offset_ms"] {
        Value::Null => 0,
        v => number(v, "offset_ms")?.round() as i64,
    };
    if !(0..=OFFSET_MAX_MS).contains(&offset_ms) {
        return Err("offset_ms must be between 0 and 86400000.".into());
    }
    Ok(View { name: if name.is_empty() { clean_name(&stem(&file)) } else { name }, file, voice, offset_ms })
}

pub fn parse_session(text: &str) -> Result<Session, String> {
    let value: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|_| "The recording's JSON file is not valid.".to_string())?;
    if !value.is_object() {
        return Err("The recording's JSON file must be an object.".into());
    }
    let string = |key: &str| match &value[key] {
        Value::Null => Ok(None),
        Value::String(s) => Ok(Some(s.clone())),
        _ => Err(format!("{key} must be a text.")),
    };
    let title = string("title")?.map(|t| clean_text(&t, TITLE_MAX)).filter(|t| !t.is_empty());
    let start_ms = match string("start")? {
        None => None,
        Some(s) => Some(parse_iso(&s).ok_or("start must be a UTC time like 2026-10-19T12:30:00Z.")?),
    };
    let views = list(&value["views"], "views must be a list.")?.iter().map(parse_view).collect::<Result<Vec<_>, _>>()?;
    views_fit(views.len())?;
    let items = list(&value["events"], "events must be a list.")?.iter().map(parse_item).collect::<Result<Vec<_>, _>>()?;
    Ok(Session { id: string("id")?.map(|i| clean_text(&i, 60)).filter(|i| !i.is_empty()), title, start_ms, views, items })
}

fn without_nulls(mut value: Value) -> Value {
    if let Some(map) = value.as_object_mut() {
        map.retain(|_, v| !v.is_null());
    }
    value
}

pub fn session_text(session: &Session) -> String {
    let views: Vec<Value> = session.views.iter().map(|v| without_nulls(json!({"name": v.name, "file": v.file, "voice": v.voice, "offset_ms": v.offset_ms}))).collect();
    let out = without_nulls(json!({"id": session.id, "title": session.title, "start": session.start_ms.map(iso), "views": views}));
    serde_json::to_string_pretty(&out).unwrap_or_default() + "\n"
}

pub fn views_from_files(files: &[String]) -> Result<Vec<View>, String> {
    let mut sorted: Vec<&String> = files.iter().filter(|f| is_media(f)).collect();
    sorted.sort_by_key(|f| (!is_video(f), f.to_lowercase()));
    if sorted.is_empty() {
        return Err("There is no video file to import.".into());
    }
    views_fit(sorted.len())?;
    let names = unique_names(&sorted.iter().map(|f| clean_name(&stem(f))).collect::<Vec<_>>());
    Ok(sorted.into_iter().zip(names).map(|(file, name)| View { name, file: file.clone(), voice: None, offset_ms: 0 }).collect())
}

pub fn resolve(session: Option<Session>, folder: &str, files: &[String]) -> Result<Session, String> {
    let mut session = session.unwrap_or_default();
    if session.views.is_empty() {
        session.views = views_from_files(files)?;
    } else {
        let names = unique_names(&session.views.iter().map(|v| v.name.clone()).collect::<Vec<_>>());
        for (view, name) in session.views.iter_mut().zip(names) {
            view.name = name;
        }
    }
    if session.title.is_none() {
        session.title = Some(clean_text(folder, TITLE_MAX)).filter(|t| !t.is_empty());
    }
    Ok(session)
}

pub fn parse_clock(text: &str) -> Option<f64> {
    let parts: Vec<&str> = text.trim().split(':').collect();
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    let mut total = 0.0;
    for (i, part) in parts.iter().enumerate() {
        let v: f64 = part.trim().parse().ok().filter(|v: &f64| v.is_finite() && *v >= 0.0)?;
        if i > 0 && v >= 60.0 {
            return None;
        }
        total = total * 60.0 + v;
    }
    Some(total)
}

fn csv_rows(text: &str) -> Result<Vec<Vec<String>>, String> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let (mut row, mut field, mut quoted, mut any) = (Vec::new(), String::new(), false, false);
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(c) = chars.next() {
        any = true;
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            ('"', true) => quoted = false,
            ('"', false) if field.is_empty() => quoted = true,
            (',', false) => row.push(std::mem::take(&mut field)),
            ('\r', false) => {}
            ('\n', false) => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
                any = false;
            }
            _ => field.push(c),
        }
    }
    if quoted {
        return Err("The notes file has a quote that never closes.".into());
    }
    if any {
        row.push(field);
        rows.push(row);
    }
    Ok(rows.into_iter().filter(|r| r.iter().any(|f| !f.trim().is_empty())).collect())
}

pub fn parse_notes(text: &str) -> Result<Vec<Item>, String> {
    let rows = csv_rows(text)?;
    let Some((head, body)) = rows.split_first() else { return Ok(Vec::new()) };
    let column = |names: &[&str]| head.iter().position(|h| names.contains(&h.trim().to_lowercase().as_str()));
    let (Some(time), text_at) = (column(&["seconds", "time", "at_s"]), column(&["text", "note"])) else {
        return Err("The notes file needs a seconds column.".into());
    };
    let (kind_at, tags_at) = (column(&["kind", "type"]), column(&["tags"]));
    let mut out = Vec::new();
    for (i, row) in body.iter().enumerate() {
        let cell = |at: Option<usize>| at.and_then(|a| row.get(a)).map(|c| c.trim().to_string()).unwrap_or_default();
        let row = format!("Row {} of the notes file", i + 2);
        let at_s = parse_clock(&cell(Some(time))).ok_or_else(|| format!("{row} has no valid time."))?;
        let text = cell(text_at);
        let kind = match cell(kind_at).to_lowercase().as_str() {
            "" if text.is_empty() => "mark".to_string(),
            "" => "note".to_string(),
            other => other.to_string(),
        };
        if kind == "chapter" {
            continue;
        }
        out.push(tag_list(&json!(cell(tags_at).split_whitespace().collect::<Vec<_>>())).and_then(|tags| item(at_s, None, &kind, &text, tags)).map_err(|e| format!("{row}: {e}"))?);
    }
    Ok(out)
}

fn csv_field(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) { format!("\"{}\"", text.replace('"', "\"\"")) } else { text.to_string() }
}

pub fn notes_text(items: &[Item]) -> String {
    let mut out = String::from("seconds,kind,text,tags\n");
    for i in items.iter().filter(|i| i.kind != "chapter") {
        out += &format!("{},{},{},{}\n", i.at_s, i.kind, csv_field(&i.text), csv_field(&i.tags.join(" ")));
    }
    out
}

pub fn vtt_time(seconds: f64) -> String {
    let ms = (seconds.max(0.0) * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02}.{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

pub fn parse_vtt(text: &str) -> Result<Vec<Cue>, String> {
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n").replace('\r', "\n");
    let mut blocks = text.split("\n\n");
    let head = blocks.next().unwrap_or_default();
    if !head.starts_with("WEBVTT") {
        return Err("A WebVTT file starts with WEBVTT.".into());
    }
    let mut out = Vec::new();
    let header_cue = head.lines().skip(1).any(|l| l.contains("-->"));
    let all: Vec<&str> = if header_cue { std::iter::once(head).chain(blocks).collect() } else { blocks.collect() };
    for block in all {
        let lines: Vec<&str> = block.lines().filter(|l| !l.starts_with("WEBVTT")).collect();
        let Some(at) = lines.iter().position(|l| l.contains("-->")) else { continue };
        if lines.first().is_some_and(|l| l.starts_with("NOTE") || l.starts_with("STYLE") || l.starts_with("REGION")) {
            continue;
        }
        let (from, rest) = lines[at].split_once("-->").unwrap_or_default();
        let to = rest.split_whitespace().next().unwrap_or_default();
        let (Some(from_s), Some(to_s)) = (parse_clock(from), parse_clock(to)) else { return Err(format!("Bad time in WebVTT line: {}", lines[at])) };
        let cue_text = crate::clean_note(&lines[at + 1..].join(" "), TEXT_MAX).trim().to_string();
        if to_s > from_s && !cue_text.is_empty() {
            out.push(Cue { from_s, to_s, text: cue_text });
        }
    }
    Ok(out)
}

pub fn vtt_text(cues: &[Cue]) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for (i, cue) in cues.iter().enumerate() {
        out += &format!("{}\n{} --> {}\n{}\n\n", i + 1, vtt_time(cue.from_s), vtt_time(cue.to_s), cue.text.replace('\n', " "));
    }
    out
}

pub fn chapter_items(cues: Vec<Cue>) -> Vec<Item> {
    cues.into_iter().map(|c| Item { at_s: tenth(c.from_s), to_s: Some(tenth(c.to_s)), kind: "chapter".into(), text: c.text, tags: Vec::new() }).collect()
}

pub fn chapter_cues(items: &[Item]) -> Vec<Cue> {
    items.iter().filter(|i| i.kind == "chapter").filter_map(|i| Some(Cue { from_s: i.at_s, to_s: i.to_s?, text: i.text.clone() })).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_utc_iso_and_round_trip() {
        assert_eq!(parse_iso("2026-10-19T12:30:00Z"), Some(1_792_413_000_000));
        assert_eq!(parse_iso("2026-10-19T12:30:00.250Z"), Some(1_792_413_000_250));
        assert_eq!(iso(1_792_413_000_250), "2026-10-19T12:30:00.250Z");
        assert_eq!(iso(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(parse_iso("2024-02-29T23:59:59Z").map(iso).as_deref(), Some("2024-02-29T23:59:59.000Z"));
        for bad in ["2026-10-19T12:30:00", "2026-10-19 12:30:00Z", "2026-13-19T12:30:00Z", "2026-10-19T24:00:00Z", "yesterday", "", "2026-10-19T12:30:00+09:00", "2026-10-19T12:30:00.xZ", "2026-10-19T-1:30:00Z"] {
            assert_eq!(parse_iso(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_smallest_session_is_a_title_and_a_start() {
        let s = parse_session("{\"title\": \"Scrim vs Team B\", \"start\": \"2026-10-19T12:30:00Z\"}").unwrap();
        assert_eq!((s.title.as_deref(), s.start_ms, s.views.len(), s.items.len()), (Some("Scrim vs Team B"), Some(1_792_413_000_000), 0, 0));
        assert_eq!(parse_session("{}").unwrap(), Session::default());
        assert_eq!(parse_session("\u{feff}{\"unknown\": [1, 2]}").unwrap(), Session::default());
    }

    #[test]
    fn views_default_their_name_file_and_offset() {
        let s = parse_session("{\"views\": [{\"file\": \"Ann.mp4\"}, {\"name\": \"Ben\"}, {\"name\": \"Coach\", \"file\": \"clips/walk.mkv\", \"voice\": \"coach voice.m4a\", \"offset_ms\": 1500}]}").unwrap();
        assert_eq!((s.views[0].name.as_str(), s.views[0].file.as_str(), s.views[0].offset_ms), ("Ann", "Ann.mp4", 0));
        assert_eq!((s.views[1].name.as_str(), s.views[1].file.as_str()), ("Ben", "Ben.mp4"));
        assert_eq!((s.views[2].file.as_str(), s.views[2].voice.as_deref(), s.views[2].offset_ms), ("clips/walk.mkv", Some("coach voice.m4a"), 1500));
    }

    #[test]
    fn malformed_sessions_are_refused_with_a_reason() {
        let bad = [
            ("not json", "is not valid"),
            ("[1]", "must be an object"),
            ("{\"title\": 3}", "title must be a text"),
            ("{\"start\": \"2026-10-19\"}", "start must be a UTC time"),
            ("{\"views\": {}}", "views must be a list"),
            ("{\"views\": [{}]}", "needs a file or a name"),
            ("{\"views\": [{\"file\": \"../x.mp4\"}]}", "inside the folder"),
            ("{\"views\": [{\"file\": \"C:/x.mp4\"}]}", "inside the folder"),
            ("{\"views\": [{\"file\": \"/x.mp4\"}]}", "inside the folder"),
            ("{\"views\": [{\"file\": \"a.mp4\", \"voice\": \"..\\\\v.m4a\"}]}", "inside the folder"),
            ("{\"views\": [{\"file\": \"a.mp4\", \"offset_ms\": -1}]}", "offset_ms"),
            ("{\"views\": [{\"file\": \"a.mp4\", \"offset_ms\": \"x\"}]}", "offset_ms must be a number"),
            ("{\"views\": [{\"file\": 4}]}", "file must be a text"),
            ("{\"events\": [{\"text\": \"x\"}]}", "at_s must be a number"),
            ("{\"events\": [{\"at_s\": -2, \"text\": \"x\"}]}", "seconds from the start"),
            ("{\"events\": [{\"at_s\": 1, \"kind\": \"note\"}]}", "needs text"),
            ("{\"events\": [{\"at_s\": 1, \"kind\": \"chapter\", \"text\": \"x\"}]}", "needs text and a to_s"),
            ("{\"events\": [{\"at_s\": 5, \"to_s\": 2, \"text\": \"x\"}]}", "needs text and a to_s"),
            ("{\"events\": [{\"at_s\": 1, \"kind\": \"drawing\", \"text\": \"x\"}]}", "kind must be"),
            ("{\"events\": [{\"at_s\": 1, \"text\": \"x\", \"tags\": 3}]}", "tags must be"),
        ];
        for (text, reason) in bad {
            let error = parse_session(text).unwrap_err();
            assert!(error.contains(reason), "{text}: {error}");
        }
        let seven = format!("{{\"views\": [{}]}}", (0..7).map(|i| format!("{{\"file\": \"{i}.mp4\"}}")).collect::<Vec<_>>().join(","));
        assert!(parse_session(&seven).unwrap_err().contains("at most 6"));
    }

    #[test]
    fn events_read_as_notes_marks_and_chapters() {
        let s = parse_session("{\"events\": [{\"at_s\": 84.04, \"text\": \"Late rotate #rotate\", \"tags\": [\"#mistake\", \"mistake\", \"rotate\"]}, {\"at_s\": 10}, {\"at_s\": 0, \"to_s\": 60, \"text\": \"Match 1\"}]}").unwrap();
        assert_eq!(s.items[0], Item { at_s: 84.0, to_s: None, kind: "note".into(), text: "Late rotate #rotate".into(), tags: vec!["mistake".into(), "rotate".into()] });
        assert_eq!((s.items[1].kind.as_str(), s.items[2].kind.as_str(), s.items[2].to_s), ("mark", "chapter", Some(60.0)));
    }

    #[test]
    fn a_session_written_is_read_back() {
        let s = Session {
            id: Some("2026-10-19 21-30-00".into()),
            title: Some("Scrim".into()),
            start_ms: Some(1_792_413_000_000),
            views: vec![View { name: "Ann".into(), file: "Ann.mp4".into(), voice: Some("Ann voice.m4a".into()), offset_ms: 0 }, View { name: "Ben".into(), file: "Ben.mp4".into(), voice: None, offset_ms: 2500 }],
            items: Vec::new(),
        };
        assert_eq!(parse_session(&session_text(&s)).unwrap(), s);
    }

    #[test]
    fn a_bare_folder_makes_one_view_per_file_named_after_it() {
        let files: Vec<String> = ["Ben.mp4", "notes.txt", "Ann.MKV", "ann.mp4", "commentary.m4a"].map(String::from).to_vec();
        let s = resolve(None, "Scrim night", &files).unwrap();
        assert_eq!(s.title.as_deref(), Some("Scrim night"));
        assert_eq!(s.views.iter().map(|v| (v.name.as_str(), v.file.as_str(), v.offset_ms)).collect::<Vec<_>>(), [("Ann", "Ann.MKV", 0), ("ann 2", "ann.mp4", 0), ("Ben", "Ben.mp4", 0), ("commentary", "commentary.m4a", 0)]);
        assert!(resolve(None, "x", &["notes.txt".to_string()]).unwrap_err().contains("no video"));
        let named = resolve(Some(Session { title: Some("Own".into()), views: vec![View { name: "A".into(), file: "a.mp4".into(), voice: None, offset_ms: 0 }, View { name: "a".into(), file: "b.mp4".into(), voice: None, offset_ms: 0 }], ..Default::default() }), "folder", &[]).unwrap();
        assert_eq!((named.title.as_deref(), named.views[1].name.as_str()), (Some("Own"), "a 2"));
    }

    #[test]
    fn file_names_are_safe_and_distinct() {
        assert_eq!(file_name("Ann/Ben: \"x\"?"), "Ann_Ben_ _x__");
        assert_eq!(file_name("..."), "view");
        assert_eq!(unique_names(&["Ann".into(), "ann".into(), "".into(), "Ann".into()]), ["Ann", "ann 2", "View", "Ann 3"]);
        assert!(safe_relative("a/b.mp4") && safe_relative("a b.mp4"));
        assert!(!safe_relative("") && !safe_relative("a//b") && !safe_relative("./a") && !safe_relative("a\u{0}b"));
    }

    #[test]
    fn exported_file_names_never_meet_the_files_beside_them() {
        let names: Vec<String> = ["Ann/Ben", "Chapters", "notes", "ann_ben", "Session"].map(String::from).to_vec();
        assert_eq!(export_names(&names), ["Ann_Ben", "Chapters view", "notes view", "ann_ben 2", "Session view"]);
    }

    #[test]
    fn notes_csv_is_read_by_column_name() {
        let csv = "\u{feff}Seconds,Kind,Text,Tags\r\n84.5,note,\"Late rotate, ring closing\",mistake rotate\r\n1:05,,,\r\n0:10:00,note,\"He said \"\"go\"\"\",\r\n\r\n";
        let items = parse_notes(csv).unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!((items[0].at_s, items[0].text.as_str(), items[0].tags.clone()), (84.5, "Late rotate, ring closing", vec!["mistake".to_string(), "rotate".to_string()]));
        assert_eq!((items[1].at_s, items[1].kind.as_str()), (65.0, "mark"));
        assert_eq!((items[2].at_s, items[2].text.as_str()), (600.0, "He said \"go\""));
        assert_eq!(parse_notes("time,text\n5,hi\n").unwrap()[0].kind, "note");
        assert!(parse_notes("").unwrap().is_empty());
    }

    #[test]
    fn malformed_notes_are_refused_with_the_row() {
        assert!(parse_notes("text\nhello\n").unwrap_err().contains("seconds column"));
        assert!(parse_notes("seconds,text\nsoon,hello\n").unwrap_err().contains("Row 2"));
        assert!(parse_notes("seconds,text\n5,ok\n-3,bad\n").unwrap_err().contains("Row 3"));
        assert!(parse_notes("seconds,kind,text\n5,note,\n").unwrap_err().contains("needs text"));
        assert!(parse_notes("seconds,kind,text\n5,video,x\n").unwrap_err().contains("kind must be"));
        assert!(parse_notes("seconds,text\n5,\"open\n").unwrap_err().contains("quote"));
    }

    #[test]
    fn notes_are_written_and_read_back() {
        let items = vec![
            Item { at_s: 84.5, to_s: None, kind: "note".into(), text: "Late, \"very\" late\nrotate".into(), tags: vec!["mistake".into()] },
            Item { at_s: 90.0, to_s: None, kind: "mark".into(), text: String::new(), tags: Vec::new() },
            Item { at_s: 0.0, to_s: Some(60.0), kind: "chapter".into(), text: "Match 1".into(), tags: Vec::new() },
        ];
        let back = parse_notes(&notes_text(&items)).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!((back[0].at_s, back[0].text.as_str()), (84.5, "Late, \"very\" late\nrotate"));
        assert_eq!(back[1], items[1]);
    }

    #[test]
    fn webvtt_chapters_and_subtitles_are_read() {
        let vtt = "\u{feff}WEBVTT - Match chapters\r\n\r\nNOTE made by hand\r\n\r\n1\r\n00:00:00.000 --> 00:10:12.300 align:start\r\nMatch 1\r\n\r\nintro\r\n01:00.000 --> 01:30.000\r\nSecond line\r\nof the cue\r\n\r\n00:00:05.000 --> 00:00:05.000\r\nempty\r\n";
        let cues = parse_vtt(vtt).unwrap();
        assert_eq!(cues, vec![Cue { from_s: 0.0, to_s: 612.3, text: "Match 1".into() }, Cue { from_s: 60.0, to_s: 90.0, text: "Second line of the cue".into() }]);
        assert!(parse_vtt("hello").unwrap_err().contains("WEBVTT"));
        assert!(parse_vtt("WEBVTT\n\nx --> y\nhi\n").unwrap_err().contains("Bad time"));
        assert!(parse_vtt("WEBVTT\n").unwrap().is_empty());
    }

    #[test]
    fn webvtt_is_written_and_read_back() {
        let cues = vec![Cue { from_s: 0.0, to_s: 612.3, text: "Match 1".into() }, Cue { from_s: 3725.5, to_s: 3730.0, text: "Drop north".into() }];
        let text = vtt_text(&cues);
        assert!(text.starts_with("WEBVTT\n\n1\n00:00:00.000 --> 00:10:12.300\nMatch 1\n"));
        assert!(text.contains("01:02:05.500 --> 01:02:10.000"));
        assert_eq!(parse_vtt(&text).unwrap(), cues);
        let items = chapter_items(cues.clone());
        assert_eq!(chapter_cues(&items), cues);
    }
}
