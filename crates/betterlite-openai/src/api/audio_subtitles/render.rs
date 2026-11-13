use super::grouping::group_tokens_into_cues;
use super::{SubtitleCue, SubtitleToken};

const MILLIS_PER_HOUR: i64 = 3_600_000;
const MILLIS_PER_MINUTE: i64 = 60_000;
const MILLIS_PER_SECOND: i64 = 1_000;

#[must_use]
pub fn format_timestamp(total_ms: i64, millis_separator: char) -> String {
    let clamped = total_ms.max(0);
    let hours = clamped.div_euclid(MILLIS_PER_HOUR);
    let hour_remainder = clamped.rem_euclid(MILLIS_PER_HOUR);
    let minutes = hour_remainder.div_euclid(MILLIS_PER_MINUTE);
    let minute_remainder = hour_remainder.rem_euclid(MILLIS_PER_MINUTE);
    let seconds = minute_remainder.div_euclid(MILLIS_PER_SECOND);
    let millis = minute_remainder.rem_euclid(MILLIS_PER_SECOND);
    format!("{hours:02}:{minutes:02}:{seconds:02}{millis_separator}{millis:03}")
}

#[must_use]
pub fn render_subtitle_tokens_as_srt(tokens: &[SubtitleToken]) -> String {
    let cues = group_tokens_into_cues(tokens);
    if cues.is_empty() {
        return String::new();
    }
    let mut lines: Vec<String> = Vec::new();
    for (number, cue) in (1_u64..).zip(&cues) {
        lines.push(number.to_string());
        push_cue_lines(&mut lines, cue, ',');
    }
    lines.join("\n")
}

#[must_use]
pub fn render_subtitle_tokens_as_vtt(tokens: &[SubtitleToken]) -> String {
    let mut lines: Vec<String> = vec![String::from("WEBVTT"), String::new()];
    for cue in &group_tokens_into_cues(tokens) {
        push_cue_lines(&mut lines, cue, '.');
    }
    lines.join("\n")
}

fn push_cue_lines(lines: &mut Vec<String>, cue: &SubtitleCue, millis_separator: char) {
    lines.push(format!(
        "{} --> {}",
        format_timestamp(cue.start_ms, millis_separator),
        format_timestamp(cue.end_ms, millis_separator)
    ));
    lines.push(cue.text.clone());
    lines.push(String::new());
}
