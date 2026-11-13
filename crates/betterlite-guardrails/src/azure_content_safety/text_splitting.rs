#[must_use]
pub fn split_text_by_words(text: &str, max_length: usize) -> Vec<String> {
    // LiteLLM loops forever on a zero limit, so a zero limit splits to single characters instead.
    let max_length = max_length.max(1);
    if text.chars().count() <= max_length {
        return vec![text.to_owned()];
    }
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_length = 0_usize;
    for token in word_and_space_runs(text) {
        let token_length = token.chars().count();
        if current_length.saturating_add(token_length) <= max_length {
            current.push_str(&token);
            current_length = current_length.saturating_add(token_length);
            continue;
        }
        if !current.is_empty() {
            chunks.push(std::mem::take(&mut current));
        }
        let mut pieces: Vec<String> = token
            .chars()
            .collect::<Vec<char>>()
            .chunks(max_length)
            .map(|piece| piece.iter().collect())
            .collect();
        let last = pieces.pop().unwrap_or_default();
        chunks.extend(pieces);
        current_length = last.chars().count();
        current = last;
    }
    // Text longer than the limit always ends with a non-empty token, so the remainder is a chunk.
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
            runs.push(std::mem::take(&mut run));
        }
        run_in_space = is_space;
        run.push(character);
    }
    runs.push(run);
    runs
}
