use std::collections::BTreeSet;
use std::ops::Div as _;

use betterlite_config::sections::content_filter_records::overrides::CompetitorIntentResult;
use regex::Regex;
use serde::Deserialize;

use super::competitor_intent_base::configuration::Settings;
use super::competitor_intent_base::{
    BaseCompetitorIntentChecker, CompetitorIntentError, Meaning, normalize,
};

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
    id: Option<String>,
    #[serde(rename = "match")]
    matches: Option<String>,
}

pub struct AirlineCompetitorIntentChecker {
    base: BaseCompetitorIntentChecker,
    other_marker: Option<Regex>,
    competitor_marker: Option<Regex>,
}

impl AirlineCompetitorIntentChecker {
    pub fn new(config: &Settings) -> Result<Self, CompetitorIntentError> {
        let mut airline_config = config.clone();
        if airline_config.competitors.is_empty() {
            airline_config.competitors = load_competitors_excl_brand(&airline_config.brand_self);
        }
        if airline_config.domain_words.is_empty() {
            airline_config.domain_words = ["airline", "airlines", "carrier"]
                .map(str::to_owned)
                .to_vec();
        }
        let base = BaseCompetitorIntentChecker::new(&airline_config)?;
        let competitor_marker = Regex::new(&format!("(?i){EXPLICIT_COMPETITOR_MARKER}")).ok();
        let other_marker = Regex::new(&format!("(?i){EXPLICIT_OTHER_MEANING_MARKER}")).ok();
        Ok(Self {
            base,
            other_marker,
            competitor_marker,
        })
    }

    #[must_use]
    pub fn run(&self, text: &str) -> CompetitorIntentResult {
        if text.is_empty() {
            return self.base.run_with(text, |normalized, token| {
                self.classify_ambiguous(normalized, token)
            });
        }
        self.base.run_with(text, |normalized, token| {
            self.classify_ambiguous(normalized, token)
        })
    }

    #[must_use]
    pub fn classify_ambiguous(&self, text: &str, token: &str) -> (Meaning, f64) {
        let normalized = normalize(text);
        if self
            .competitor_marker
            .as_ref()
            .is_some_and(|marker| marker.is_match(&normalized) && word_boundary(&normalized, token))
        {
            return (Meaning::Competitor, 0.85);
        }
        if self
            .other_marker
            .as_ref()
            .is_some_and(|marker| marker.is_match(&normalized))
        {
            return (Meaning::OtherMeaning, 0.85);
        }
        let has_comparison = count_signals(&normalized, COMPARISON_SIGNALS) > 0;
        if !has_comparison && count_signals(&normalized, OPERATIONAL_SIGNALS) > 0 {
            return (Meaning::OtherMeaning, 0.85);
        }
        score_context(
            count_signals(&normalized, AIRLINE_OTHER_MEANING_SIGNALS),
            count_signals(&normalized, AIRLINE_COMPETITOR_SIGNALS),
        )
    }
}

pub fn load_competitors_excl_brand(brand_self: &[String]) -> Vec<String> {
    let Ok(airlines) = serde_json::from_str::<Vec<AirlineEntry>>(AIRLINES_JSON) else {
        return Vec::new();
    };
    let brands: BTreeSet<String> = brand_self
        .iter()
        .map(|brand| normalize(brand))
        .filter(|brand| !brand.is_empty())
        .collect();
    let mut competitors = Vec::new();
    for airline in airlines {
        let variants = airline
            .matches
            .unwrap_or_default()
            .split('|')
            .map(normalize)
            .filter(|variant| !variant.is_empty())
            .collect::<Vec<_>>();
        let id = airline
            .id
            .map(|value| normalize(&value))
            .unwrap_or_default();
        let overlaps = brands.contains(&id)
            || variants.iter().any(|variant| {
                brands.contains(variant)
                    || variant.split_whitespace().any(|word| brands.contains(word))
            });
        if !overlaps {
            competitors.extend(variants);
        }
    }
    competitors
}

fn count_signals(text: &str, patterns: &[&str]) -> usize {
    let mut count = 0_usize;
    for pattern in patterns {
        if Regex::new(&format!("(?i){pattern}")).is_ok_and(|regex| regex.is_match(text)) {
            count = count.saturating_add(1);
        }
    }
    count
}

fn score_context(other_count: usize, competitor_count: usize) -> (Meaning, f64) {
    let total = other_count.saturating_add(competitor_count);
    if total == 0 {
        return (Meaning::OtherMeaning, 0.5_f64);
    }
    let total_ratio = f64::from(u32::try_from(total).unwrap_or(u32::MAX));
    let other_ratio = f64::from(u32::try_from(other_count).unwrap_or(u32::MAX)).div(total_ratio);
    let competitor_ratio =
        f64::from(u32::try_from(competitor_count).unwrap_or(u32::MAX)).div(total_ratio);
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
#[path = "competitor_intent_airline/tests/signal_scoring.rs"]
mod signal_scoring;
