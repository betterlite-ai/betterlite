use regex::Regex;
use unicode_normalization::UnicodeNormalization as _;

#[must_use]
pub fn normalize(text: &str) -> String {
    let without_zero_width: String = text
        .chars()
        .filter(|character| !matches!(character, '\u{200b}'..='\u{200d}' | '\u{2060}' | '\u{feff}'))
        .collect();
    let normalized: String = without_zero_width.nfkc().collect();
    let lowercased = normalized.to_lowercase();
    let leet_replaced = lowercased.chars().map(|character| match character {
        '@' | '4' => 'a',
        '0' => 'o',
        '3' => 'e',
        '1' => 'i',
        '5' => 's',
        '7' => 't',
        other => other,
    });
    leet_replaced
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[must_use]
pub fn text_for_entity_matching(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_alphanumeric() || character.is_whitespace() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn word_boundary_match(text: &str, token: &str) -> bool {
    let pattern = format!(r"\b{}\b", regex::escape(token));
    Regex::new(&pattern).is_ok_and(|regex| regex.is_match(text))
}

pub(crate) fn count_signals(text: &str, patterns: &[String]) -> usize {
    patterns
        .iter()
        .filter_map(|pattern| compile_marker(Some(pattern)))
        .filter(|pattern| pattern.is_match(text))
        .count()
}

pub(crate) fn compile_marker(pattern: Option<&str>) -> Option<Regex> {
    let value = pattern.filter(|value| !value.trim().is_empty())?;
    Regex::new(&format!("(?i){value}")).ok()
}
