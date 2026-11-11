#[must_use]
pub fn convert_raw_bytes_to_str_lines(raw_bytes: &[Vec<u8>]) -> Vec<String> {
    let text = lossy_text(&raw_bytes.concat());
    // Python's str.strip and Rust's str::trim both take Unicode whitespace, so one trim covers both ends.
    text.split('\n')
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

// from_utf8_lossy is disallowed in this crate, so the replacement decode is built from utf8_chunks.
fn lossy_text(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        text.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            text.push(char::REPLACEMENT_CHARACTER);
        }
    }
    text
}
