use serde::{Deserialize, Serialize};

use super::tier::Tier;

pub const DEFAULT_MATCH_THRESHOLD: f64 = 0.5;

pub const DEFAULT_KEYWORD_TIER_RULES: [(&[&str], Tier); 4] = [
    (&["hi", "hello", "thanks"], Tier::Simple),
    (&["explain", "how does"], Tier::Medium),
    (&["refactor", "implement", "debug"], Tier::Complex),
    (&["step by step", "think through", "prove"], Tier::Reasoning),
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
pub fn default_keywords(tier: Tier) -> &'static [&'static str] {
    DEFAULT_KEYWORD_TIER_RULES
        .iter()
        .find_map(|(keywords, ruled_tier)| (*ruled_tier == tier).then_some(*keywords))
        .unwrap_or_default()
}
