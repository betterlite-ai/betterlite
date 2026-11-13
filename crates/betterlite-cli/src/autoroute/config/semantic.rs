use serde::{Deserialize, Serialize};

use super::tier::Tier;

pub const DEFAULT_MATCH_THRESHOLD: f64 = 0.5;

const SIMPLE_KEYWORDS: &[&str] = &["hi", "hello", "thanks"];
const MEDIUM_KEYWORDS: &[&str] = &["explain", "how does"];
const COMPLEX_KEYWORDS: &[&str] = &["refactor", "implement", "debug"];
const REASONING_KEYWORDS: &[&str] = &["step by step", "think through", "prove"];

// a const cannot hold KeywordTierRule's Vec<String>, so each tier pairs with its keyword slice
pub const DEFAULT_KEYWORD_TIER_RULES: [(&[&str], Tier); 4] = [
    (SIMPLE_KEYWORDS, Tier::Simple),
    (MEDIUM_KEYWORDS, Tier::Medium),
    (COMPLEX_KEYWORDS, Tier::Complex),
    (REASONING_KEYWORDS, Tier::Reasoning),
];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KeywordTierRule {
    pub keywords: Vec<String>,
    pub tier: Tier,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SemanticMatching {
    pub embedding_model: String,
    pub match_threshold: f64,
    pub keyword_tier_rules: Vec<KeywordTierRule>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SemanticMatchingChoice {
    Disabled,
    Enabled(SemanticMatching),
}

#[must_use]
pub const fn default_keywords(tier: Tier) -> &'static [&'static str] {
    match tier {
        Tier::Simple => SIMPLE_KEYWORDS,
        Tier::Medium => MEDIUM_KEYWORDS,
        Tier::Complex => COMPLEX_KEYWORDS,
        Tier::Reasoning => REASONING_KEYWORDS,
    }
}
