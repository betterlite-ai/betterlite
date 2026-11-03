use std::ops::Range;

use unicode_general_category::{GeneralCategory, get_general_category};

use super::PatternError;
use super::spans::{DIGIT_RANGES, MAX_TEXT_BYTES};

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
    let normalized = format::string_padding(format);
    let uppercase = pattern_name.to_uppercase();
    let mut remaining = MAX_TEXT_BYTES.saturating_sub(format.len());
    Ok(strfmt::strfmt_map(&normalized, |mut field| {
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

mod format;
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
        let position =
            decimal_index(index).ok_or_else(|| strfmt::FmtError::KeyError(key.to_owned()))?;
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

fn decimal_index(index: &str) -> Option<usize> {
    if index.is_empty() {
        return None;
    }
    index.chars().try_fold(0_usize, |value, character| {
        if get_general_category(character) != GeneralCategory::DecimalNumber {
            return None;
        }
        // The pinned ranges exclude decimal characters from newer Unicode versions.
        let (start, _) = DIGIT_RANGES
            .iter()
            .find(|(start, end)| (*start..=*end).contains(&character))?;
        // Consecutive mathematical digit alphabets each restart at zero.
        let digit = u32::from(character)
            .checked_sub(u32::from(*start))?
            .checked_rem(10)?;
        value
            .checked_mul(10)?
            .checked_add(usize::try_from(digit).ok()?)
    })
}
