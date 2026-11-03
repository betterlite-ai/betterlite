use std::collections::VecDeque;

use super::{MAX_SPLIT_OUTPUT_BYTES, SplitError};

pub(super) fn select_separator<'sep>(
    text: &str,
    separators: &'sep [String],
) -> (&'sep str, &'sep [String]) {
    for (index, candidate) in separators.iter().enumerate() {
        if candidate.is_empty() {
            return (candidate, &[]);
        }
        if text.contains(candidate.as_str()) {
            let rest_start = index.saturating_add(1);
            return (candidate, separators.get(rest_start..).unwrap_or(&[]));
        }
    }
    (separators.last().map_or("", String::as_str), &[])
}

pub(super) fn push_joined(
    chunks: &mut Vec<String>,
    total_bytes: &mut usize,
    current_chunk: &VecDeque<&str>,
    separator: &str,
) -> Result<(), SplitError> {
    let joined = current_chunk
        .iter()
        .copied()
        .collect::<Vec<_>>()
        .join(separator);
    let trimmed = joined.trim_matches(is_strip_whitespace);
    if !trimmed.is_empty() {
        push_chunk(chunks, total_bytes, trimmed.to_owned())?;
    }
    Ok(())
}

pub(super) fn push_chunk(
    chunks: &mut Vec<String>,
    total_bytes: &mut usize,
    chunk: String,
) -> Result<(), SplitError> {
    let next = total_bytes
        .checked_add(chunk.len())
        .ok_or(SplitError::OutputTooLarge {
            limit: MAX_SPLIT_OUTPUT_BYTES,
        })?;
    if next > MAX_SPLIT_OUTPUT_BYTES {
        return Err(SplitError::OutputTooLarge {
            limit: MAX_SPLIT_OUTPUT_BYTES,
        });
    }
    *total_bytes = next;
    chunks.push(chunk);
    Ok(())
}

pub(super) fn char_slices(text: &str) -> Vec<&str> {
    let mut result = Vec::new();
    for (start, ch) in text.char_indices() {
        let end = start.saturating_add(ch.len_utf8());
        if let Some(slice) = text.get(start..end) {
            result.push(slice);
        }
    }
    result
}

pub(super) fn chunk_by_chars(text: &str, chunk_size: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut result = Vec::new();
    let mut index = 0_usize;
    while index < chars.len() {
        let end = index.saturating_add(chunk_size).min(chars.len());
        result.push(chars.get(index..end).unwrap_or_default().iter().collect());
        index = index.saturating_add(chunk_size);
    }
    result
}

// str.strip()'s whitespace set also covers U+001C-U+001F; Rust's is_whitespace() does not.
pub(super) fn is_strip_whitespace(candidate: char) -> bool {
    if candidate.is_whitespace() {
        true
    } else {
        ('\u{1c}'..='\u{1f}').contains(&candidate)
    }
}
