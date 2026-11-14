pub(super) fn char_boundary(text: &str, index: usize) -> usize {
    text.char_indices()
        .nth(index)
        .map_or(text.len(), |(offset, _character)| offset)
}

// Python slices and measures the streamed buffer by characters, never by bytes
pub(super) fn char_slice(text: &str, start: usize, end: usize) -> &str {
    let start_byte = char_boundary(text, start);
    let end_byte = char_boundary(text, end);
    text.get(start_byte..end_byte).unwrap_or_default()
}
