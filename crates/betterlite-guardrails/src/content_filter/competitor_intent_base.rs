use std::collections::{BTreeMap, BTreeSet};

mod classification;
pub mod configuration;
mod matching;
use regex::Regex;
use unicode_normalization::UnicodeNormalization as _;

const OTHER_MEANING_THRESHOLD: f64 = 0.65;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Meaning {
    Competitor,
    OtherMeaning,
}
#[derive(Debug, Clone)]
pub struct BaseCompetitorIntentChecker {
    pub brand_self: Vec<String>,
    pub competitor_canonical: BTreeMap<String, String>,
    pub competitor_tokens: BTreeSet<String>,
    pub ambiguous: BTreeSet<String>,
    pub policy: BTreeMap<String, String>,
    pub threshold_high: f64,
    pub threshold_medium: f64,
    pub threshold_low: f64,
    pub reframe_message_template: Option<String>,
    pub refuse_message_template: Option<String>,
    pub comparison_words: Vec<String>,
    pub domain_words: Vec<String>,
}

#[path = "competitor_intent_base/classify_ambiguous/tests.rs"]
mod classify_ambiguous_tests;
#[path = "competitor_intent_base/compile_marker/tests.rs"]
mod compile_marker_tests;
#[path = "competitor_intent_base/count_signals/tests.rs"]
mod count_signals_tests;
#[path = "competitor_intent_base/find_matches/tests.rs"]
mod find_matches_tests;
#[path = "competitor_intent_base/init/tests.rs"]
mod init_tests;
#[path = "competitor_intent_base/normalize/tests.rs"]
mod normalize_tests;
#[path = "competitor_intent_base/run/tests.rs"]
mod run_tests;
mod tests;
#[path = "competitor_intent_base/text_for_entity_matching/tests.rs"]
mod text_for_entity_matching_tests;
#[path = "competitor_intent_base/word_boundary/tests.rs"]
mod word_boundary_tests;

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
        '\u{1c}'..='\u{1f}' => ' ',
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
    let characters = text.chars();
    characters
        .map(|character| {
            if character.is_alphanumeric() || character.is_whitespace() || character == '_' {
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

fn word_boundary_match(text: &str, token: &str) -> bool {
    let pattern = format!(r"\b{}\b", regex::escape(token));
    compile_marker(Some(&format!("(?-i:{pattern})"))).is_some_and(|regex| regex.is_match(text))
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("competitor {competitor:?} uses token {token:?} already mapped to {existing:?}")]
    TokenCollision {
        competitor: String,
        token: String,
        existing: String,
    },
    #[error("invalid signal pattern")]
    Pattern {
        #[source]
        source: regex::Error,
    },
}

fn count_signals(text: &str, patterns: &[String]) -> Result<usize, Error> {
    let matches = patterns.iter().map(|pattern| {
        Regex::new(&format!("(?i){pattern}"))
            .map(|regex| usize::from(regex.is_match(text)))
            .map_err(|source| Error::Pattern { source })
    });
    matches.sum()
}

fn has_words(text: &str, words: &[String]) -> bool {
    let patterns: Vec<String> = words
        .iter()
        .map(|word| format!(r"(?-i:\b{}\b)", regex::escape(word)))
        .collect();
    count_signals(text, &patterns).is_ok_and(|count| count > 0)
}

fn compile_marker(pattern: Option<&str>) -> Option<Regex> {
    let value = pattern.filter(|value| !value.trim().is_empty())?;
    Regex::new(&format!("(?i){value}")).ok()
}
