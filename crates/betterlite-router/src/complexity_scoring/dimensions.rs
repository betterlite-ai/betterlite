use std::collections::BTreeSet;

use regex::Regex;

use crate::complexity_math::{FULL, HALF, LOW, NEGATIVE, NONE, ratio};
use crate::complexity_scoring::config::TokenThresholds;
use crate::complexity_scoring::error::ScoringError;
use crate::complexity_scoring::keywords::{KeywordList, compile_insensitive_patterns};

pub const CUSTOM_PATTERN_SCAN_CHARS: usize = 2048;

// the pinned matchers keep one entry per lowercase keyword and per pattern, so a repeat counts once
fn dedupe_lowered(keywords: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut deduped = Vec::with_capacity(keywords.len());
    for keyword in keywords {
        if seen.insert(keyword.to_lowercase()) {
            deduped.push(keyword.clone());
        }
    }
    deduped
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Dimension {
    TokenCount,
    CodePresence,
    ReasoningMarkers,
    TechnicalTerms,
    SimpleIndicators,
    MultiStepPatterns,
    QuestionComplexity,
    Custom(String),
}

impl Dimension {
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::TokenCount => "tokenCount",
            Self::CodePresence => "codePresence",
            Self::ReasoningMarkers => "reasoningMarkers",
            Self::TechnicalTerms => "technicalTerms",
            Self::SimpleIndicators => "simpleIndicators",
            Self::MultiStepPatterns => "multiStepPatterns",
            Self::QuestionComplexity => "questionComplexity",
            Self::Custom(name) => name.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DimensionScore {
    pub dimension: Dimension,
    pub score: f64,
    pub signal: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScoringMode {
    Binary,
    MatchCount,
}

impl ScoringMode {
    const fn saturation(self) -> usize {
        match self {
            Self::Binary => 1,
            Self::MatchCount => 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CustomDimension {
    pub name: String,
    pub weight: f64,
    pub keywords: Vec<String>,
    pub patterns: Vec<String>,
    pub scoring_mode: ScoringMode,
}

pub struct CustomDimensionMatcher {
    name: String,
    weight: f64,
    keywords: KeywordList,
    patterns: Vec<Regex>,
    saturation: usize,
}

impl CustomDimensionMatcher {
    pub fn compile(dimension: &CustomDimension) -> Result<Self, ScoringError> {
        // a repeated pattern counts once, matched on the exact pattern text
        let mut seen = BTreeSet::new();
        let patterns: Vec<String> = dimension
            .patterns
            .iter()
            .filter(|pattern| seen.insert((*pattern).clone()))
            .cloned()
            .collect();
        Ok(Self {
            name: dimension.name.clone(),
            weight: dimension.weight,
            keywords: KeywordList::compile(&dedupe_lowered(&dimension.keywords))?,
            patterns: compile_insensitive_patterns(&patterns)?,
            saturation: dimension.scoring_mode.saturation(),
        })
    }

    // _count_custom_hits: distinct matchers, keywords on the ask and patterns on its first 2048 characters
    fn hits(&self, user_text: &str, scanned: &str) -> usize {
        let keyword_hits = self.keywords.matches(user_text).len();
        let pattern_hits = self
            .patterns
            .iter()
            .filter(|pattern| pattern.is_match(scanned))
            .count();
        keyword_hits
            .saturating_add(pattern_hits)
            .min(self.saturation)
    }
}

pub struct KeywordRule {
    pub dimension: Dimension,
    pub label: &'static str,
    pub low_threshold: usize,
    pub high_threshold: usize,
    pub score_none: f64,
    pub score_low: f64,
    pub score_high: f64,
}

pub const CODE_PRESENCE: KeywordRule = KeywordRule {
    dimension: Dimension::CodePresence,
    label: "code",
    low_threshold: 1,
    high_threshold: 2,
    score_none: NONE,
    score_low: HALF,
    score_high: FULL,
};

pub const REASONING_MARKERS: KeywordRule = KeywordRule {
    dimension: Dimension::ReasoningMarkers,
    label: "reasoning",
    low_threshold: 1,
    high_threshold: 2,
    score_none: NONE,
    score_low: LOW,
    score_high: FULL,
};

pub const TECHNICAL_TERMS: KeywordRule = KeywordRule {
    dimension: Dimension::TechnicalTerms,
    label: "technical",
    low_threshold: 2,
    high_threshold: 4,
    score_none: NONE,
    score_low: HALF,
    score_high: FULL,
};

pub const SIMPLE_INDICATORS: KeywordRule = KeywordRule {
    dimension: Dimension::SimpleIndicators,
    label: "simple",
    low_threshold: 1,
    high_threshold: 2,
    score_none: NONE,
    score_low: NEGATIVE,
    score_high: NEGATIVE,
};

#[must_use]
pub fn estimated_tokens(text: &str) -> i64 {
    let characters = text.chars().count();
    let tokens = characters.checked_div(4).unwrap_or(0);
    i64::try_from(tokens).unwrap_or(i64::MAX)
}

#[must_use]
pub fn score_token_count(tokens: i64, thresholds: TokenThresholds) -> DimensionScore {
    let (score, signal) = if tokens < thresholds.simple {
        (NEGATIVE, Some(format!("short ({tokens} tokens)")))
    } else if tokens > thresholds.complex {
        (FULL, Some(format!("long ({tokens} tokens)")))
    } else {
        (NONE, None)
    };
    DimensionScore {
        dimension: Dimension::TokenCount,
        score,
        signal,
    }
}

#[must_use]
pub fn score_keyword_match(
    text: &str,
    keywords: &KeywordList,
    rule: &KeywordRule,
) -> (DimensionScore, usize) {
    let matches = keywords.matches(text);
    let count = matches.len();
    let score = if count < rule.low_threshold {
        rule.score_none
    } else if count >= rule.high_threshold {
        rule.score_high
    } else {
        rule.score_low
    };
    let signal = (count >= rule.low_threshold).then(|| {
        let detail = matches
            .iter()
            .take(3)
            .copied()
            .collect::<Vec<_>>()
            .join(", ");
        format!("{} ({detail})", rule.label)
    });
    (
        DimensionScore {
            dimension: rule.dimension.clone(),
            score,
            signal,
        },
        count,
    )
}

#[must_use]
pub fn score_multi_step(text: &str, patterns: &[Regex]) -> DimensionScore {
    let hit = patterns.iter().any(|pattern| pattern.is_match(text));
    DimensionScore {
        dimension: Dimension::MultiStepPatterns,
        score: if hit { HALF } else { NONE },
        signal: hit.then(|| "multi-step".to_owned()),
    }
}

#[must_use]
pub fn score_question_complexity(text: &str) -> DimensionScore {
    let count = text.matches('?').count();
    DimensionScore {
        dimension: Dimension::QuestionComplexity,
        score: if count > 3 { HALF } else { NONE },
        signal: (count > 3).then(|| format!("{count} questions")),
    }
}

#[must_use]
pub fn score_custom_dimensions(
    prompt: &str,
    user_text: &str,
    matchers: &[CustomDimensionMatcher],
) -> Vec<(DimensionScore, f64)> {
    if matchers.is_empty() {
        return Vec::new();
    }
    let scanned: String = prompt.chars().take(CUSTOM_PATTERN_SCAN_CHARS).collect();
    let mut scored = Vec::new();
    for matcher in matchers {
        let hits = matcher.hits(user_text, &scanned);
        if hits == 0 {
            continue;
        }
        scored.push((
            DimensionScore {
                dimension: Dimension::Custom(matcher.name.clone()),
                score: ratio(hits, matcher.saturation),
                signal: Some(format!("custom ({})", matcher.name)),
            },
            matcher.weight,
        ));
    }
    scored
}
