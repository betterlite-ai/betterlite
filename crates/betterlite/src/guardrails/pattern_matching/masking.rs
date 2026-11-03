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
        if field.key != "pattern_name" {
            return Err(strfmt::FmtError::KeyError(field.key.to_owned()));
        }
        let maximum = field
            .width()
            .unwrap_or(0)
            .saturating_mul(4)
            .max(uppercase.len());
        remaining = remaining
            .checked_sub(maximum)
            .ok_or_else(|| strfmt::FmtError::Invalid("redaction exceeds size limit".to_owned()))?;
        field.str(&uppercase)
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
