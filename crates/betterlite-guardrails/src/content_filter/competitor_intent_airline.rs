use std::collections::BTreeSet;
use std::ops::Div as _;

use betterlite_config::sections::content_filter_records::overrides::CompetitorIntentResult;
use regex::Regex;
use serde::Deserialize;

use super::competitor_intent_base::{
    BaseCompetitorIntentChecker, CompetitorIntentError, Meaning, normalize,
};

pub mod settings;
use settings::AirlineSettings;

pub const AIRLINE_OTHER_MEANING_SIGNALS: &[&str] = &[
    r"\b(fly|flying|travel|traveling|going|visit|visiting|transit|layover|stopover)\b.{0,12}\b(to|from|via|in|at|through|into)\b",
    r"\bflight(s)?\b.{0,10}\b(to|from|via)\b",
    r"\bticket(s)?\b.{0,8}\b(to|for)\b",
    r"\bfare(s)?\b.{0,8}\b(to)\b",
    r"\bvisa\b",
    r"\bimmigration\b",
    r"\bcustoms\b",
    r"\bentry\b",
    r"\bairport\b",
    r"\bterminal\b",
    r"\bgate\b",
    r"\bdeparture\b",
    r"\barrival\b",
    r"\bitinerary\b",
    r"\bweather\b",
    r"\bhotel\b",
    r"\bcity\b",
    r"\bto\s+",
    r"\bfrom\s+",
    r"\bin\s+",
    r"\bat\s+",
    r"\bvia\s+",
];

pub const AIRLINE_COMPETITOR_SIGNALS: &[&str] = &[
    r"\bairways?\b",
    r"\bairline\b",
    r"\bcarrier\b",
    r"\bcabin\s+crew\b",
    r"\bflight\s+attendant\b",
    r"\bbusiness\s+class\b",
    r"\bfirst\s+class\b",
    r"\beconomy\b",
    r"\blounge\b",
    r"\bbaggage\s+allowance\b",
    r"\bcheck[- ]?in\b",
    r"\bmiles\b",
    r"\bloyalty\b",
    r"\bstatus\b",
    r"\bfrequent\s+flyer\b",
    r"\bfleet\b",
    r"\baircraft\b",
    r"\bbetter\b",
    r"\bbest\b",
    r"\bgood\b",
    r"\bas\s+good\s+as\b",
    r"\bvs\.?\b",
    r"\bversus\b",
    r"\bcompare\b",
    r"\balternative\b",
    r"\bcompetitor\b",
    r"\bqmiles\b",
    r"\bprivilege\s+club\b",
];

const OPERATIONAL_SIGNALS: &[&str] = &[
    r"\bbaggage\s+allowance\b",
    r"\blounge\b",
    r"\bcheck[- ]?in\b",
    r"\brefund\b",
    r"\bpremium\s+lounge\b",
];
const COMPARISON_SIGNALS: &[&str] = &[
    r"\bbetter\b",
    r"\bbest\b",
    r"\bvs\.?\b",
    r"\bversus\b",
    r"\bcompare\b",
];
const EXPLICIT_COMPETITOR_MARKER: &str = r"\b(airways?|airline|carrier)\b";
const EXPLICIT_OTHER_MEANING_MARKER: &str =
    r"\b(fly|travel|going|visit|layover|stopover|transit)\b.{0,12}\b(to|in|via|from)\b.{0,8}\b";
const AIRLINES_JSON: &str = include_str!("competitor_intent/major_airlines.json");

#[derive(Debug, Deserialize)]
struct AirlineEntry {
    #[serde(rename = "match")]
    matches: Option<String>,
}

pub struct AirlineCompetitorIntentChecker {
    base: BaseCompetitorIntentChecker,
    other_marker: Option<Regex>,
    competitor_marker: Option<Regex>,
    other_meaning_signals: Vec<String>,
    competitor_signals: Vec<String>,
    other_meaning_anchors: Vec<String>,
}

impl AirlineCompetitorIntentChecker {
    pub fn new(settings: &AirlineSettings) -> Result<Self, CompetitorIntentError> {
        let mut base_config = settings.base.clone();
        if base_config.competitors.is_empty() {
            base_config.competitors = load_competitors_excl_brand(&base_config.brand_self);
        }
        if base_config.domain_words.is_empty() {
            base_config.domain_words = ["airline", "airlines", "carrier"]
                .map(str::to_owned)
                .to_vec();
        }
        let base = BaseCompetitorIntentChecker::new(&base_config)?;
        let other_meaning_signals = owned_or_default(
            &settings.other_meaning_signals,
            AIRLINE_OTHER_MEANING_SIGNALS,
        );
        let competitor_signals =
            owned_or_default(&settings.competitor_signals, AIRLINE_COMPETITOR_SIGNALS);
        let competitor_marker = compile_explicit_marker(
            settings.explicit_competitor_marker.as_deref(),
            EXPLICIT_COMPETITOR_MARKER,
        );
        let other_marker = compile_explicit_marker(
            settings.explicit_other_meaning_marker.as_deref(),
            EXPLICIT_OTHER_MEANING_MARKER,
        );
        Ok(Self {
            base,
            other_marker,
            competitor_marker,
            other_meaning_signals,
            competitor_signals,
            other_meaning_anchors: settings.other_meaning_anchors.clone(),
        })
    }

    #[must_use]
    pub fn run(&self, text: &str) -> CompetitorIntentResult {
        let normalized = normalize(text);
        self.base.run_with(&normalized, |segment, token| {
            self.classify_ambiguous(segment, token)
        })
    }

    #[must_use]
    pub fn classify_ambiguous(&self, text: &str, token: &str) -> (Meaning, f64) {
        let text_lower = text.to_lowercase();
        let token_lower = token.to_lowercase();
        if self.competitor_marker.as_ref().is_some_and(|marker| {
            marker.is_match(&text_lower) && word_boundary(&text_lower, &token_lower)
        }) {
            return (Meaning::Competitor, 0.85_f64);
        }
        if self
            .other_marker
            .as_ref()
            .is_some_and(|marker| marker.is_match(&text_lower))
        {
            return (Meaning::OtherMeaning, 0.85_f64);
        }
        let has_comparison = count_signals(&text_lower, COMPARISON_SIGNALS) > 0;
        if !has_comparison && count_signals(&text_lower, OPERATIONAL_SIGNALS) > 0 {
            return (Meaning::OtherMeaning, 0.85_f64);
        }
        let other_count =
            count_signals(&text_lower, self.other_meaning_signals.as_slice()).saturating_add(
                count_signals(&text_lower, self.other_meaning_anchors.as_slice()),
            );
        let competitor_count = count_signals(&text_lower, self.competitor_signals.as_slice());
        score_context(other_count, competitor_count)
    }
}

fn owned_or_default(custom: &[String], default_patterns: &[&str]) -> Vec<String> {
    if custom.is_empty() {
        default_patterns
            .iter()
            .map(|pattern| (*pattern).to_owned())
            .collect()
    } else {
        custom.to_vec()
    }
}

fn compile_explicit_marker(custom: Option<&str>, default_pattern: &str) -> Option<Regex> {
    let pattern = custom
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(default_pattern);
    Regex::new(&format!("(?i){pattern}")).ok()
}

#[must_use]
pub fn load_competitors_excl_brand(brand_self: &[String]) -> Vec<String> {
    let Ok(airlines) = serde_json::from_str::<Vec<AirlineEntry>>(AIRLINES_JSON) else {
        return Vec::new();
    };
    let brands: BTreeSet<String> = brand_self
        .iter()
        .map(|brand| brand.trim().to_lowercase())
        .filter(|brand| !brand.is_empty())
        .collect();
    let mut competitors = Vec::new();
    for airline in airlines {
        let variants = airline
            .matches
            .unwrap_or_default()
            .split('|')
            .map(|variant| variant.trim().to_lowercase())
            .filter(|variant| !variant.is_empty())
            .collect::<Vec<_>>();
        let overlaps = variants.iter().any(|variant| {
            brands.contains(variant) || variant.split_whitespace().any(|word| brands.contains(word))
        });
        if !overlaps {
            competitors.extend(variants);
        }
    }
    competitors
}

fn count_signals<S>(text: &str, patterns: &[S]) -> u32
where
    S: AsRef<str>,
{
    let mut count = 0_u32;
    for pattern in patterns {
        if Regex::new(&format!("(?i){}", pattern.as_ref())).is_ok_and(|regex| regex.is_match(text))
        {
            count = count.saturating_add(1);
        }
    }
    count
}

fn score_context(other_count: u32, competitor_count: u32) -> (Meaning, f64) {
    let total = other_count.saturating_add(competitor_count);
    if total == 0 {
        return (Meaning::OtherMeaning, 0.5_f64);
    }
    let total_f64 = f64::from(total);
    let other_ratio = f64::from(other_count).div(total_f64);
    let competitor_ratio = f64::from(competitor_count).div(total_f64);
    if other_ratio >= 0.6_f64 {
        return (
            Meaning::OtherMeaning,
            0.4_f64.mul_add(other_ratio, 0.5_f64).min(0.9_f64),
        );
    }
    if competitor_ratio >= 0.6_f64 {
        return (
            Meaning::Competitor,
            0.4_f64.mul_add(competitor_ratio, 0.5_f64).min(0.9_f64),
        );
    }
    (Meaning::OtherMeaning, 0.5_f64)
}

fn word_boundary(text: &str, token: &str) -> bool {
    if let Ok(pattern) = Regex::new(&format!(r"(?i)\b{}\b", regex::escape(token))) {
        return pattern.is_match(text);
    }
    false
}

#[path = "competitor_intent_airline/tests/action_hint.rs"]
mod action_hint;
#[path = "competitor_intent_airline/tests/airline_classify.rs"]
mod airline_classify;
#[path = "competitor_intent_airline/tests/airline_init.rs"]
mod airline_init;
#[path = "competitor_intent_airline/tests/ambiguous_token.rs"]
mod ambiguous_token;
#[path = "competitor_intent_airline/tests/category_ranking_confidence.rs"]
mod category_ranking_confidence;
#[path = "competitor_intent_airline/tests/classify_ambiguous_base.rs"]
mod classify_ambiguous_base;
#[path = "competitor_intent_airline/tests/confidence_with_comparison.rs"]
mod confidence_with_comparison;
#[path = "competitor_intent_airline/tests/confidence_without_comparison.rs"]
mod confidence_without_comparison;
#[path = "competitor_intent_airline/tests/evidence.rs"]
mod evidence;
#[path = "competitor_intent_airline/tests/find_matches.rs"]
mod find_matches;
#[path = "competitor_intent_airline/tests/load_competitors.rs"]
mod load_competitors;
#[path = "competitor_intent_airline/tests/run_category_ranking.rs"]
mod run_category_ranking;
#[path = "competitor_intent_airline/tests/run_competitor_comparison.rs"]
mod run_competitor_comparison;
#[path = "competitor_intent_airline/tests/run_log_only.rs"]
mod run_log_only;
#[path = "competitor_intent_airline/tests/run_non_competitor.rs"]
mod run_non_competitor;
#[path = "competitor_intent_airline/tests/run_possible_competitor.rs"]
mod run_possible_competitor;
#[path = "competitor_intent_airline/tests/shared.rs"]
mod shared;
#[path = "competitor_intent_airline/tests/signal_scoring.rs"]
mod signal_scoring;
