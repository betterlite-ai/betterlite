use super::DEFAULT_CHUNK_SIZE_BYTES;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextChunk {
    pub char_offset: usize,
    pub text: String,
}

// json.dumps escapes non-ASCII, so a 3-byte UTF-8 character takes 6 bytes (12 outside the BMP) in the request body.
pub fn json_escaped_len<I>(characters: I) -> usize
where
    I: IntoIterator<Item = char>,
{
    characters
        .into_iter()
        .map(|character| match character {
            '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
            '\u{0}'..='\u{1f}' => 6,
            '\u{20}'..='\u{7f}' => 1,
            _ if character.len_utf16() == 1 => 6,
            _ => 12,
        })
        .sum()
}

#[must_use]
pub fn coerce_analyze_chunk_size(value: Option<i64>) -> usize {
    match value {
        Some(size) if size > 0 => usize::try_from(size).unwrap_or(usize::MAX).max(4),
        _ => DEFAULT_CHUNK_SIZE_BYTES,
    }
}

// low stays at 1 so a character wider than a tiny budget still becomes a chunk of its own
#[must_use]
pub fn largest_fitting_prefix(candidate: &[char], budget_bytes: usize) -> usize {
    let mut low = 1;
    let mut high = candidate.len();
    while low < high {
        let middle = low.saturating_add(high).saturating_add(1) >> 1_u32;
        let prefix = candidate.get(..middle).unwrap_or_default();
        if json_escaped_len(prefix.iter().copied()) <= budget_bytes {
            low = middle;
        } else {
            high = middle.saturating_sub(1);
        }
    }
    low
}

#[must_use]
pub fn split_text_for_analysis(
    text: &str,
    chunk_size_bytes: usize,
    overlap_chars: usize,
) -> Vec<TextChunk> {
    let characters: Vec<char> = text.chars().collect();
    let mut chunks = Vec::new();
    let mut start = 0_usize;
    while start < characters.len() {
        let window_end = start.saturating_add(chunk_size_bytes).min(characters.len());
        let candidate = characters.get(start..window_end).unwrap_or_default();
        let fitting = if json_escaped_len(candidate.iter().copied()) <= chunk_size_bytes {
            candidate.len()
        } else {
            largest_fitting_prefix(candidate, chunk_size_bytes)
        };
        let chunk = candidate.get(..fitting).unwrap_or_default();
        let end = start.saturating_add(chunk.len());
        chunks.push(TextChunk {
            char_offset: start,
            text: chunk.iter().collect(),
        });
        if end >= characters.len() {
            break;
        }
        let overlap = overlap_chars.min(chunk.len() >> 1_u32);
        start = start.saturating_add(1).max(end.saturating_sub(overlap));
    }
    chunks
}
