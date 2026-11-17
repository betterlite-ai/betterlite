use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use num_traits::ToPrimitive as _;
use serde_json::Number;

// Python's str.splitlines() breaks on every one of these, including the separators a provider never sends
pub(crate) const fn is_line_boundary(character: char) -> bool {
    matches!(
        character,
        '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{1c}'..='\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

// Python's str.strip() also strips the ASCII separators, which char::is_whitespace leaves in place
pub(crate) fn trim_line(text: &str) -> &str {
    text.trim_matches(|character: char| {
        character.is_whitespace() || matches!(character, '\u{1c}'..='\u{1f}')
    })
}

// _chunk_index: an absent or unusable index falls back to the argument the caller passes
pub(crate) fn chunk_index(parsed: &IndexMap<String, Json>, key: &str, fallback: i64) -> i64 {
    let Some(value) = parsed.get(key) else {
        return fallback;
    };
    index_of(value).unwrap_or(fallback)
}

fn index_of(value: &Json) -> Option<i64> {
    match value {
        Json::Bool(flag) => Some(i64::from(*flag)),
        Json::Number(number) => number_index(number),
        Json::Text(text) => index_text(text),
        Json::Null | Json::List(_) | Json::Object(_) => None,
    }
}

// Python's int() truncates a float toward zero; a number outside i64 cannot index anything here
fn number_index(number: &Number) -> Option<i64> {
    if let Some(integer) = number.as_i64() {
        return Some(integer);
    }
    number.as_f64()?.to_i64()
}

// Python's int(str): surrounding whitespace, one sign, and single underscores between ASCII digits
fn index_text(text: &str) -> Option<i64> {
    let trimmed = trim_line(text);
    let (sign, digits) = trimmed.strip_prefix('-').map_or_else(
        || ("", trimmed.strip_prefix('+').unwrap_or(trimmed)),
        |rest| ("-", rest),
    );
    let mut cleaned = String::with_capacity(digits.len());
    let mut previous_digit = false;
    for character in digits.chars() {
        if character.is_ascii_digit() {
            cleaned.push(character);
            previous_digit = true;
        } else if character == '_' && previous_digit {
            previous_digit = false;
        } else {
            return None;
        }
    }
    if cleaned.is_empty() || !previous_digit {
        return None;
    }
    format!("{sign}{cleaned}").parse::<i64>().ok()
}
