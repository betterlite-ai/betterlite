use std::mem;

#[must_use]
pub fn split_text_by_words(text: &str, max_length: usize) -> Vec<String> {
    // LiteLLM loops forever on a zero limit, so a zero limit splits to single characters instead.
    let chunk_limit = max_length.max(1);
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_length = 0_usize;
    for token in word_and_space_runs(text) {
        let token_length = token.chars().count();
        if current_length.saturating_add(token_length) <= chunk_limit {
            current.push_str(&token);
            current_length = current_length.saturating_add(token_length);
            continue;
        }
        if !current.is_empty() {
            chunks.push(mem::take(&mut current));
        }
        let mut pieces: Vec<String> = token
            .chars()
            .collect::<Vec<char>>()
            .chunks(chunk_limit)
            .map(|piece| piece.iter().collect())
            .collect();
        let last = pieces.pop().unwrap_or_default();
        chunks.extend(pieces);
        current_length = last.chars().count();
        current = last;
    }
    // Every token is non-empty, so the remainder after the last one is always a chunk.
    chunks.push(current);
    chunks
}

fn word_and_space_runs(text: &str) -> Vec<String> {
    let mut runs = Vec::new();
    let mut run = String::new();
    let mut run_in_space = false;
    for character in text.chars() {
        // Python's `\s` follows str.isspace(), which also counts the U+001C to U+001F separators.
        let is_space = character.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&character);
        if run_in_space != is_space && !run.is_empty() {
            runs.push(mem::take(&mut run));
        }
        run_in_space = is_space;
        run.push(character);
    }
    runs.push(run);
    runs
}
