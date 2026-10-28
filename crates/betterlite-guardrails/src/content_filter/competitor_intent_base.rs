use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::content_filter_records::overrides::{
    CompetitorIntentConfig, CompetitorIntentEvidenceEntry, CompetitorIntentResult,
};

#[path = "evidence.rs"]
mod evidence;
#[path = "normalization.rs"]
mod normalization;
use evidence::{empty_entities, entity_evidence, other_result, signal_evidence};
pub(super) use normalization::{compile_marker, count_signals, word_boundary_match};
use normalization::{
    count_signals as count_signals_impl, word_boundary_match as word_boundary_match_impl,
};
pub use normalization::{normalize, text_for_entity_matching};

const OTHER_MEANING_THRESHOLD: f64 = 0.65;
const DEFAULT_COMPARISON_WORDS: [&str; 9] = [
    "better",
    "worse",
    "best",
    "vs",
    "versus",
    "compare",
    "alternative",
    "recommend",
    "ranked",
];

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

impl BaseCompetitorIntentChecker {
    #[must_use]
    pub fn new(config: &CompetitorIntentConfig) -> Self {
        let brand_self = config
            .brand_self
            .iter()
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty())
            .collect();
        let (competitor_canonical, competitor_tokens) = build_competitor_maps(config);
        let other_meaning = normalized_set(&config.route_geo_cues);
        let ambiguous = competitor_tokens
            .intersection(&other_meaning)
            .cloned()
            .collect();
        Self {
            brand_self,
            competitor_canonical,
            competitor_tokens,
            ambiguous,
            policy: config.policy.clone(),
            threshold_high: config.threshold_high.unwrap_or(0.70),
            threshold_medium: config.threshold_medium.unwrap_or(0.45),
            threshold_low: config.threshold_low.unwrap_or(0.30),
            reframe_message_template: config.reframe_message_template.clone(),
            refuse_message_template: config.refuse_message_template.clone(),
            comparison_words: DEFAULT_COMPARISON_WORDS
                .iter()
                .map(|word| (*word).to_owned())
                .collect(),
            domain_words: config
                .domain_words
                .iter()
                .map(|word| word.trim().to_lowercase())
                .collect(),
        }
    }

    #[must_use]
    pub fn classify_ambiguous(&self, _text: &str, _token: &str) -> (String, f64) {
        let label = "OTHER_MEANING".to_owned();
        let confidence = 0.5;
        (label, confidence)
    }

    #[must_use]
    pub fn find_matches(&self, text: &str) -> Vec<(String, String, bool)> {
        let normalized = normalize(text);
        self.competitor_tokens
            .iter()
            .filter(|token| word_boundary_match_impl(&normalized, token))
            .filter_map(|token| {
                self.competitor_canonical.get(token).map(|canonical| {
                    (
                        token.clone(),
                        canonical.clone(),
                        self.ambiguous.contains(token),
                    )
                })
            })
            .collect()
    }

    #[must_use]
    pub fn run(&self, text: &str) -> CompetitorIntentResult {
        let normalized = normalize(text);
        let (mut entities, mut evidence) = self.brand_entities(&normalized);
        let matches = self.find_matches(text);
        if matches.is_empty() {
            return self.category_or_other(&normalized, entities, evidence);
        }
        let (mut competitors, match_evidence) = self.resolve_matches(&normalized, matches);
        evidence.extend(match_evidence);
        self.classify_resolved(&normalized, &mut entities, evidence, &mut competitors)
    }

    fn classify_resolved(
        &self,
        normalized: &str,
        entities: &mut BTreeMap<String, Vec<String>>,
        mut evidence: Vec<CompetitorIntentEvidenceEntry>,
        competitors: &mut Vec<String>,
    ) -> CompetitorIntentResult {
        competitors.sort();
        competitors.dedup();
        entities.insert("competitors".to_owned(), competitors.clone());
        if competitors.is_empty() {
            return other_result(entities, evidence);
        }
        let has_comparison = count_signals_impl(normalized, &self.comparison_words) > 0;
        let confidence: f64 = if has_comparison { 0.75 } else { 0.55 };
        let intent = intent_for_confidence(
            confidence,
            self.threshold_high,
            self.threshold_medium,
            self.threshold_low,
        );
        if has_comparison {
            evidence.push(signal_evidence("comparison", "comparison language"));
        }
        let action_hint = match intent {
            "log_only" => "log_only",
            "other" => "allow",
            _ => self.policy.get(intent).map_or("allow", String::as_str),
        };
        CompetitorIntentResult {
            intent: Some(intent.to_owned()),
            confidence: Some(confidence),
            entities: entities.clone(),
            signals: if has_comparison {
                vec!["competitor_resolved".to_owned(), "comparison".to_owned()]
            } else {
                vec!["competitor_resolved".to_owned()]
            },
            action_hint: Some(action_hint.to_owned()),
            evidence,
        }
    }

    fn brand_entities(
        &self,
        normalized: &str,
    ) -> (
        BTreeMap<String, Vec<String>>,
        Vec<CompetitorIntentEvidenceEntry>,
    ) {
        let mut entities = empty_entities();
        let mut evidence = Vec::new();
        for brand in &self.brand_self {
            if word_boundary_match_impl(normalized, brand) {
                entities.insert("brand_self".to_owned(), vec![brand.clone()]);
                evidence.push(entity_evidence("brand_self", brand, brand));
            }
        }
        (entities, evidence)
    }

    fn resolve_matches(
        &self,
        normalized: &str,
        matches: Vec<(String, String, bool)>,
    ) -> (Vec<String>, Vec<CompetitorIntentEvidenceEntry>) {
        let mut competitors = Vec::new();
        let mut evidence = Vec::new();
        for (token, canonical, is_ambiguous) in matches {
            if !is_ambiguous {
                competitors.push(canonical.clone());
                evidence.push(entity_evidence("competitor", &canonical, &token));
                continue;
            }
            let (label, confidence) = self.classify_ambiguous(normalized, &token);
            if label == "COMPETITOR" && confidence >= OTHER_MEANING_THRESHOLD {
                competitors.push(canonical.clone());
                evidence.push(entity_evidence("competitor", &canonical, &token));
            } else if label == "COMPETITOR" {
                evidence.push(signal_evidence(
                    "other_meaning_default",
                    &format!("confidence {confidence:.2}"),
                ));
            } else {
                evidence.push(signal_evidence("other_meaning", &token));
            }
        }
        (competitors, evidence)
    }

    fn category_or_other(
        &self,
        text: &str,
        entities: BTreeMap<String, Vec<String>>,
        mut evidence: Vec<CompetitorIntentEvidenceEntry>,
    ) -> CompetitorIntentResult {
        let has_comparison = count_signals_impl(text, &self.comparison_words) > 0;
        let has_domain = count_signals_impl(text, &self.domain_words) > 0;
        if has_comparison && has_domain {
            evidence.push(signal_evidence("category_ranking", "comparison + domain"));
            let action_hint = self
                .policy
                .get("category_ranking")
                .map_or("reframe", String::as_str);
            return CompetitorIntentResult {
                intent: Some("category_ranking".to_owned()),
                confidence: Some(0.65),
                entities,
                signals: vec!["category_ranking".to_owned()],
                action_hint: Some(action_hint.to_owned()),
                evidence,
            };
        }
        CompetitorIntentResult {
            intent: Some("other".to_owned()),
            confidence: Some(0.0),
            entities,
            signals: Vec::new(),
            action_hint: Some("allow".to_owned()),
            evidence,
        }
    }
}

fn normalized_set(values: &[String]) -> BTreeSet<String> {
    let normalized = values
        .iter()
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty())
        .collect();
    normalized
}

fn intent_for_confidence(confidence: f64, high: f64, medium: f64, low: f64) -> &'static str {
    if confidence >= high {
        "competitor_comparison"
    } else if confidence >= medium {
        "possible_competitor_comparison"
    } else if confidence >= low {
        "log_only"
    } else {
        "other"
    }
}

fn build_competitor_maps(
    config: &CompetitorIntentConfig,
) -> (BTreeMap<String, String>, BTreeSet<String>) {
    let mut canonical_map = BTreeMap::new();
    let mut tokens = BTreeSet::new();
    for competitor in &config.competitors {
        let canonical = competitor.trim().to_lowercase();
        if canonical.is_empty() {
            continue;
        }
        tokens.insert(canonical.clone());
        canonical_map.insert(canonical.clone(), canonical.clone());
        if let Some(alias) = config
            .competitor_aliases
            .get(competitor)
            .map(|alias| alias.trim().to_lowercase())
            .filter(|alias| !alias.is_empty())
        {
            tokens.insert(alias.clone());
            canonical_map.insert(alias, canonical);
        }
    }
    (canonical_map, tokens)
}

#[cfg(test)]
#[path = "competitor_intent_base/tests.rs"]
mod classify_ambiguous;
#[cfg(test)]
#[path = "competitor_intent_base/tests.rs"]
mod compile_marker;
#[cfg(test)]
#[path = "competitor_intent_base/tests.rs"]
mod count_signals;
#[cfg(test)]
#[path = "competitor_intent_base/tests.rs"]
mod find_matches;
#[cfg(test)]
#[path = "competitor_intent_base/tests.rs"]
mod init;
#[cfg(test)]
#[path = "competitor_intent_base/tests.rs"]
mod normalize;
#[cfg(test)]
#[path = "competitor_intent_base/tests.rs"]
mod run;
#[cfg(test)]
#[path = "competitor_intent_base/tests.rs"]
mod text_for_entity_matching;
#[cfg(test)]
#[path = "competitor_intent_base/tests.rs"]
mod word_boundary;
