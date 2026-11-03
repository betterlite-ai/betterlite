use std::ops::Range;

use super::{PatternError, spans::MAX_TEXT_BYTES};

#[derive(Debug, thiserror::Error)]
pub enum MaskError {
    #[error("invalid redaction format: {0}")]
    Format(#[from] strfmt::FmtError),
    #[error("redaction format exceeds the size limit")]
    TooLarge,
}

pub fn redaction_tag(format: &str, pattern_name: &str) -> Result<String, MaskError> {
    if format.len() > MAX_TEXT_BYTES || pattern_name.len() > MAX_TEXT_BYTES {
        return Err(MaskError::TooLarge);
    }
    let uppercase = pattern_name.to_uppercase();
    let mut remaining = MAX_TEXT_BYTES.saturating_sub(format.len());
    Ok(strfmt::strfmt_map(format, |mut field| {
        let value = indexed_name(field.key, &uppercase)?;
        let maximum = field
            .width()
            .unwrap_or(0)
            .saturating_mul(4)
            .max(value.len());
        remaining = remaining
            .checked_sub(maximum)
            .ok_or_else(|| strfmt::FmtError::Invalid("redaction exceeds size limit".to_owned()))?;
        field.str(value)
    })?)
}

pub fn mask_spans(text: &str, spans: &[Range<usize>], tag: &str) -> Result<String, PatternError> {
    if text
        .len()
        .saturating_add(spans.len().saturating_mul(tag.len()))
        > MAX_TEXT_BYTES
    {
        return Err(PatternError::TooLarge);
    }
    let mut masked = String::new();
    let mut previous = 0;
    for span in spans {
        if span.start > span.end || span.end > text.len() {
            return Err(PatternError::InvalidSpan);
        }
        masked.push_str(
            text.get(previous..span.start)
                .ok_or(PatternError::InvalidSpan)?,
        );
        masked.push_str(tag);
        previous = span.end;
    }
    masked.push_str(text.get(previous..).ok_or(PatternError::InvalidSpan)?);
    Ok(masked)
}

#[must_use]
pub fn keyword_pattern(keyword: &str) -> String {
    let mut escaped = String::new();
    for character in keyword.chars() {
        match character {
            '*' => escaped.push_str(".?"),
            '(' | ')' | '[' | ']' | '{' | '}' | '?' | '+' | '-' | '|' | '^' | '$' | '\\' | '.'
            | '&' | '~' | '#' | ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c' => {
                escaped.push('\\');
                escaped.push(character);
            }
            other => escaped.push(other),
        }
    }
    escaped
}

mod tests;

fn indexed_name<'name>(key: &str, uppercase: &'name str) -> Result<&'name str, strfmt::FmtError> {
    let mut suffix = key
        .strip_prefix("pattern_name")
        .ok_or_else(|| strfmt::FmtError::KeyError(key.to_owned()))?;
    let mut value = uppercase;
    while !suffix.is_empty() {
        let (index, rest) = suffix
            .strip_prefix('[')
            .and_then(|tail| tail.split_once(']'))
            .ok_or_else(|| strfmt::FmtError::KeyError(key.to_owned()))?;
        if index.is_empty() || !index.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(strfmt::FmtError::KeyError(key.to_owned()));
        }
        let position = index
            .parse::<usize>()
            .map_err(|_source| strfmt::FmtError::KeyError(key.to_owned()))?;
        let (start, character) = value
            .char_indices()
            .nth(position)
            .ok_or_else(|| strfmt::FmtError::KeyError(key.to_owned()))?;
        value = value
            .get(start..)
            .and_then(|tail| tail.get(..character.len_utf8()))
            .ok_or_else(|| strfmt::FmtError::KeyError(key.to_owned()))?;
        suffix = rest;
    }
    Ok(value)
}
