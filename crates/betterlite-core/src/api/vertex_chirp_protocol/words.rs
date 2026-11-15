use caseless::default_case_fold_str;

#[must_use]
pub fn normalize_word(word: &str) -> String {
    default_case_fold_str(
        &word
            .chars()
            .filter(|character| character.is_alphanumeric())
            .collect::<String>(),
    )
}

#[must_use]
pub fn new_words(previous: &str, current: &str) -> String {
    let previous_words: Vec<&str> = previous.split_whitespace().collect();
    let current_words: Vec<&str> = current.split_whitespace().collect();
    let common = previous_words
        .iter()
        .zip(&current_words)
        .position(|(old, new)| normalize_word(old) != normalize_word(new))
        .unwrap_or_else(|| previous_words.len().min(current_words.len()));
    let appended = current_words.get(common..).unwrap_or_default().join(" ");
    if appended.is_empty() {
        return String::new();
    }
    if common == 0 {
        return appended;
    }
    format!(" {appended}")
}

#[must_use]
pub fn join_transcript(committed: &str, tail: &str) -> String {
    match (committed.is_empty(), tail.is_empty()) {
        (true, true) => String::new(),
        (false, true) => committed.to_owned(),
        (true, false) => tail.to_owned(),
        (false, false) => format!("{committed} {tail}"),
    }
}
