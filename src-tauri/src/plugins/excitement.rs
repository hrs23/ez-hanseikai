use std::collections::BTreeMap;

use super::{sound, Found, Job, Kind, Output, Plugin};

pub const PLUGIN: Plugin = Plugin { id: "excitement", name: "Excitement detection", summary: "Marks where a member's voice suddenly gets loud.", sample: "", version: "1.0.0", kind: Kind::Analysis(run), options: &[] };

const RATE: u32 = 8000;
const STEP_S: f64 = 0.5;
const QUIET_DB: f64 = -100.0;
const NOISE_SHARE: f64 = 0.1;
const VOICE_OVER_NOISE_DB: f64 = 10.0;
const VOICE_MIN_S: f64 = 10.0;
const LOUD_OVER_VOICE_DB: f64 = 12.0;
const LOUD_MIN_S: f64 = 1.0;
const GAP_S: f64 = 30.0;

pub fn level(samples: &[i16]) -> f64 {
    let power = samples.iter().map(|&s| (f64::from(s) / 32768.0).powi(2)).sum::<f64>() / samples.len().max(1) as f64;
    if power > 0.0 { (10.0 * power.log10()).max(QUIET_DB) } else { QUIET_DB }
}

fn at_share(sorted: &[f64], share: f64) -> f64 {
    sorted[((sorted.len() - 1) as f64 * share).round() as usize]
}

pub fn loud_level(levels: &[f64]) -> Option<f64> {
    if levels.is_empty() {
        return None;
    }
    let mut all = levels.to_vec();
    all.sort_by(f64::total_cmp);
    let noise = at_share(&all, NOISE_SHARE);
    let voiced: Vec<f64> = all.into_iter().filter(|l| *l >= noise + VOICE_OVER_NOISE_DB).collect();
    ((voiced.len() as f64) * STEP_S >= VOICE_MIN_S).then(|| at_share(&voiced, 0.5) + LOUD_OVER_VOICE_DB)
}

pub fn peaks(levels: &[f64]) -> Vec<f64> {
    let Some(loud) = loud_level(levels) else { return Vec::new() };
    let hold = (LOUD_MIN_S / STEP_S).ceil() as usize;
    let mut out: Vec<f64> = Vec::new();
    let mut run = 0;
    for (i, l) in levels.iter().enumerate() {
        run = if *l >= loud { run + 1 } else { 0 };
        let t = (i + 1).saturating_sub(hold) as f64 * STEP_S;
        if run == hold && out.last().is_none_or(|last| t - last >= GAP_S) {
            out.push(t);
        }
    }
    out
}

fn run(job: &Job, progress: &mut dyn FnMut(f64)) -> Result<Output, String> {
    if job.voice.parts.is_empty() {
        return Ok(Output::default());
    }
    let mut levels = Vec::new();
    sound(job, &job.voice, RATE, (f64::from(RATE) * STEP_S) as usize, |samples| {
        levels.push(level(samples));
        if job.duration_s > 0.0 {
            progress(levels.len() as f64 * STEP_S / job.duration_s);
        }
    })?;
    let events = peaks(&levels)
        .into_iter()
        .map(|t| Found { kind: "excitement".into(), at_s: Some(job.voice.start_s + t), text: "Excitement".into(), tags: vec!["excitement".into()], ..Default::default() })
        .collect();
    Ok(Output { events, tags: BTreeMap::from([("excitement".to_string(), "Excitement".to_string())]) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn talk(seconds: f64, db: f64) -> Vec<f64> {
        vec![db; (seconds / STEP_S) as usize]
    }

    fn chat(parts: &[(f64, f64)]) -> Vec<f64> {
        parts.iter().flat_map(|&(s, db)| talk(s, db)).collect()
    }

    #[test]
    fn the_level_of_a_window_is_its_loudness_in_decibels() {
        assert_eq!(level(&[0; 100]), QUIET_DB);
        assert!((level(&[i16::MAX, i16::MIN]) - 0.0).abs() < 0.01);
        assert!((level(&[3277, -3277]) - -20.0).abs() < 0.01);
    }

    #[test]
    fn a_shout_well_over_the_members_usual_voice_is_marked_where_it_starts() {
        let levels = chat(&[(20.0, -70.0), (30.0, -30.0), (10.0, -70.0), (2.0, -12.0), (20.0, -30.0)]);
        assert_eq!(peaks(&levels), vec![60.0]);
    }

    #[test]
    fn the_usual_voice_follows_the_microphone_so_a_quiet_member_is_judged_by_their_own_level() {
        let levels = chat(&[(20.0, -90.0), (30.0, -50.0), (10.0, -90.0), (2.0, -32.0), (20.0, -50.0)]);
        assert_eq!(peaks(&levels), vec![60.0]);
    }

    #[test]
    fn talking_starting_after_silence_or_a_short_pop_is_not_a_shout() {
        assert!(peaks(&chat(&[(20.0, -70.0), (30.0, -30.0), (20.0, -70.0), (30.0, -30.0)])).is_empty());
        assert!(peaks(&chat(&[(20.0, -70.0), (30.0, -30.0), (0.5, -5.0), (20.0, -30.0)])).is_empty());
    }

    #[test]
    fn too_little_voice_marks_nothing() {
        assert!(peaks(&chat(&[(60.0, -70.0), (5.0, -10.0)])).is_empty());
        assert!(peaks(&[]).is_empty());
    }

    #[test]
    fn shouts_closer_than_the_gap_count_once_and_a_long_shout_once() {
        let levels = chat(&[(20.0, -70.0), (30.0, -30.0), (2.0, -12.0), (5.0, -30.0), (2.0, -12.0), (40.0, -30.0), (40.0, -12.0), (30.0, -30.0)]);
        assert_eq!(peaks(&levels), vec![50.0, 99.0]);
    }
}
