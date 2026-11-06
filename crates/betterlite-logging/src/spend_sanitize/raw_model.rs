use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use unicode_properties::{GeneralCategory, UnicodeGeneralCategory as _};

use super::UNKNOWN_MODEL;
use super::truncate::TRUNCATION_DB_SAFEGUARD_NOTE;
use super::value::{SpendMapping, to_json};
use crate::python_json::dumps;

const SCRUBBED_FIELDS: [&str; 2] = ["error_message", "traceback"];
const MARKER_PREFIX: &str = "... (litellm_truncated skipped ";

#[must_use]
pub fn scrub_raw_model_text(text: &str, raw_model: &str) -> String {
    if raw_model.is_empty() {
        return text.to_owned();
    }
    let spellings = spellings(raw_model);
    let whole = spellings
        .iter()
        .fold(text.to_owned(), |scrubbed, spelling| {
            scrubbed.replace(spelling.as_str(), UNKNOWN_MODEL)
        });
    split_by_truncation_marker(&whole, &spellings)
}

#[must_use]
pub fn scrub_raw_model_from_errors(information: &SpendMapping, raw_model: &str) -> Json {
    let mut scrubbed = IndexMap::new();
    for (key, value) in &information.entries {
        let entry = match (value.as_text(), SCRUBBED_FIELDS.contains(&key.as_str())) {
            (Some(text), true) => Json::Text(scrub_raw_model_text(text, raw_model)),
            _ => to_json(value),
        };
        scrubbed.insert(key.clone(), entry);
    }
    Json::Object(scrubbed)
}

fn spellings(raw_model: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for spelling in [
        raw_model.to_owned(),
        repr_body(raw_model),
        json_body(raw_model),
    ] {
        if !found.contains(&spelling) {
            found.push(spelling);
        }
    }
    found
}

fn json_body(text: &str) -> String {
    let dumped = dumps(&Json::Text(text.to_owned()));
    let body = dumped
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .map(str::to_owned);
    body.unwrap_or(dumped)
}

// repr() keeps printable characters, escapes the chosen quote and uses \x, \u and \U for the rest
fn repr_body(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut body = String::new();
    for character in text.chars() {
        if character == quote || character == '\\' {
            body.push('\\');
            body.push(character);
            continue;
        }
        match character {
            '\n' => body.push_str("\\n"),
            '\r' => body.push_str("\\r"),
            '\t' => body.push_str("\\t"),
            _ if is_printable(character) => body.push(character),
            _ => push_unprintable(&mut body, character),
        }
    }
    body
}

fn is_printable(character: char) -> bool {
    if character == ' ' {
        return true;
    }
    !matches!(
        character.general_category(),
        GeneralCategory::Control
            | GeneralCategory::Format
            | GeneralCategory::Surrogate
            | GeneralCategory::PrivateUse
            | GeneralCategory::Unassigned
            | GeneralCategory::LineSeparator
            | GeneralCategory::ParagraphSeparator
            | GeneralCategory::SpaceSeparator
    )
}

fn push_unprintable(body: &mut String, character: char) {
    let code = u32::from(character);
    let escaped = match code {
        0..=0xFF => format!("\\x{code:02x}"),
        0x100..=0xFFFF => format!("\\u{code:04x}"),
        _ => format!("\\U{code:08x}"),
    };
    body.push_str(&escaped);
}

fn split_by_truncation_marker(text: &str, spellings: &[String]) -> String {
    let Some((start, end)) = find_marker(text) else {
        return text.to_owned();
    };
    let head = text.get(..start).unwrap_or_default();
    let tail = text.get(end..).unwrap_or_default();
    let marker = text.get(start..end).unwrap_or_default();
    let head_cut = spellings
        .iter()
        .map(|spelling| overlap_at_end(head, spelling))
        .max()
        .unwrap_or(0);
    let tail_cut = spellings
        .iter()
        .map(|spelling| overlap_at_start(tail, spelling))
        .max()
        .unwrap_or(0);
    let kept_head = head
        .chars()
        .take(head.chars().count().saturating_sub(head_cut))
        .collect::<String>();
    let kept_tail = tail.chars().skip(tail_cut).collect::<String>();
    let head_model = if head_cut == 0 { "" } else { UNKNOWN_MODEL };
    let tail_model = if tail_cut == 0 { "" } else { UNKNOWN_MODEL };
    format!("{kept_head}{head_model}{marker}{tail_model}{kept_tail}")
}

fn find_marker(text: &str) -> Option<(usize, usize)> {
    let tail = format!(" chars. {TRUNCATION_DB_SAFEGUARD_NOTE}) ...");
    for (start, _) in text.match_indices(MARKER_PREFIX) {
        let digits_start = start.saturating_add(MARKER_PREFIX.len());
        let rest = text.get(digits_start..)?;
        let mut digits_end = None;
        for (offset, character) in rest.char_indices() {
            if character.general_category() != GeneralCategory::DecimalNumber {
                break;
            }
            let after_digit = digits_start
                .saturating_add(offset)
                .saturating_add(character.len_utf8());
            digits_end = Some(after_digit);
        }
        let Some(stop) = digits_end else { continue };
        let after = text.get(stop..)?;
        if after.starts_with(&tail) {
            return Some((start, stop.saturating_add(tail.len())));
        }
    }
    None
}

fn overlap_at_end(text: &str, spelling: &str) -> usize {
    let text_chars = text.chars().collect::<Vec<_>>();
    let spelling_chars = spelling.chars().collect::<Vec<_>>();
    longest_end_prefix_overlap(&text_chars, &spelling_chars)
}

// A start overlap is an end overlap once both strings are read backwards
fn overlap_at_start(text: &str, spelling: &str) -> usize {
    let reversed_text = text.chars().rev().collect::<Vec<_>>();
    let reversed_spelling = spelling.chars().rev().collect::<Vec<_>>();
    longest_end_prefix_overlap(&reversed_text, &reversed_spelling)
}

fn longest_end_prefix_overlap(text: &[char], spelling: &[char]) -> usize {
    let available = text.len().min(spelling.len().saturating_sub(1));
    (1..=available)
        .rev()
        .find(|&length| {
            text.iter()
                .skip(text.len().saturating_sub(length))
                .eq(spelling.iter().take(length))
        })
        .unwrap_or(0)
}
