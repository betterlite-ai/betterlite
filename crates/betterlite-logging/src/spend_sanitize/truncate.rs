pub const TRUNCATED_PAYLOAD_FIELD: &str = "litellm_truncated";

pub const TRUNCATION_DB_SAFEGUARD_NOTE: &str = "Truncation is a DB storage safeguard. Full, untruncated data is logged to logging callbacks (OTEL, Datadog, etc.). To increase the truncation limit, set `MAX_STRING_LENGTH_PROMPT_IN_DB` in your env.";

const START_PERCENT: i128 = 35;
const END_PERCENT: i128 = 65;
const PERCENT: i128 = 100;

#[must_use]
pub fn truncate_text(text: &str, max_string_length: i64) -> String {
    let length = i128::try_from(text.chars().count()).unwrap_or(i128::MAX);
    let maximum = i128::from(max_string_length);
    if length <= maximum {
        return text.to_owned();
    }
    let start_chars = maximum
        .saturating_mul(START_PERCENT)
        .saturating_div(PERCENT);
    let mut end_chars = maximum.saturating_mul(END_PERCENT).saturating_div(PERCENT);
    let total_keep = start_chars.saturating_add(end_chars);
    if total_keep > maximum {
        end_chars = maximum.saturating_sub(start_chars);
    }
    let skipped_chars = length.saturating_sub(total_keep);
    let head = slice_chars(text, 0, start_chars, length);
    let tail = slice_chars(text, end_chars.saturating_neg(), length, length);
    format!(
        "{head}... ({TRUNCATED_PAYLOAD_FIELD} skipped {skipped_chars} chars. {TRUNCATION_DB_SAFEGUARD_NOTE}) ...{tail}"
    )
}

// Python resolves a negative index from the end and clamps the result to the string
fn slice_bound(position: i128, length: i128) -> usize {
    let resolved = if position >= 0 {
        position
    } else {
        length.saturating_add(position)
    };
    usize::try_from(resolved.max(0).min(length)).unwrap_or(usize::MAX)
}

// text[from:to], where a zero to counts as the start and so keeps the whole tail
fn slice_chars(text: &str, from: i128, to: i128, length: i128) -> String {
    let start = slice_bound(from, length);
    let stop = slice_bound(to, length);
    text.chars()
        .skip(start)
        .take(stop.saturating_sub(start))
        .collect()
}
