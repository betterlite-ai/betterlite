#[path = "similarity/tests.rs"]
mod tests;

use rust_decimal::Decimal;

use super::matcher::{matching_characters, ratio};

#[must_use]
pub fn check_user_input_similarity(
    keywords: &[String],
    user_input: &str,
    threshold: Decimal,
) -> bool {
    let lowered: Vec<char> = user_input.to_lowercase().chars().collect();
    for keyword in keywords {
        let keyword_characters: Vec<char> = keyword.chars().collect();
        if keyword_characters.is_empty() || lowered.len() < keyword_characters.len() {
            continue;
        }
        for window in lowered.windows(keyword_characters.len()) {
            let matched = matching_characters(window, &keyword_characters);
            let limit = threshold.checked_mul(Decimal::from(
                window.len().saturating_add(keyword_characters.len()),
            ));
            let hit = limit.is_some_and(|bound| Decimal::from(matched.saturating_mul(2)) > bound);
            if hit {
                tracing::info!(
                    keyword = %keyword,
                    ratio = %ratio(window, &keyword_characters),
                    "rejected a prompt similar to a prompt injection keyword"
                );
                return true;
            }
        }
    }
    false
}
