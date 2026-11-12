#[path = "similarity/tests.rs"]
mod tests;

use std::time::Instant;

use rust_decimal::Decimal;

use super::error::PromptInjectionError;
use super::matcher::{matching_characters, ratio};

pub fn check_user_input_similarity(
    keywords: &[String],
    user_input: &str,
    threshold: Decimal,
    deadline: Option<Instant>,
) -> Result<bool, PromptInjectionError> {
    let lowered: Vec<char> = user_input.to_lowercase().chars().collect();
    for keyword in keywords {
        let keyword_characters: Vec<char> = keyword.chars().collect();
        if keyword_characters.is_empty() || lowered.len() < keyword_characters.len() {
            continue;
        }
        for window in lowered.windows(keyword_characters.len()) {
            if deadline.is_some_and(|bound| Instant::now() >= bound) {
                return Err(PromptInjectionError::HeuristicsTimeout);
            }
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
                return Ok(true);
            }
        }
    }
    Ok(false)
}
