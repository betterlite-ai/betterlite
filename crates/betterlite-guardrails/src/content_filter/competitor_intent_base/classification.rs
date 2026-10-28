use super::configuration::Settings;
use super::matching::{build_competitor_maps, find_matches, normalized_set, resolve_matches};
use super::{
    BaseCompetitorIntentChecker, Error, Meaning, has_words, normalize, word_boundary_match,
};
use betterlite_config::sections::content_filter_records::overrides::{
    CompetitorIntentEvidenceEntry, CompetitorIntentResult,
};
use std::collections::BTreeMap;

impl BaseCompetitorIntentChecker {
    pub fn new(config: &Settings) -> Result<Self, Error> {
        let brand_self = config
            .brand_self
            .iter()
            .filter(|value| !value.is_empty())
            .map(|value| value.trim().to_lowercase())
            .collect();
        let (competitor_canonical, competitor_tokens) = build_competitor_maps(config)?;
        let other_meaning = normalized_set(&config.locations);
        let ambiguous = competitor_tokens
            .intersection(&other_meaning)
            .cloned()
            .collect();
        Ok(Self {
            brand_self,
            competitor_canonical,
            competitor_tokens,
            ambiguous,
            policy: config.policy.clone(),
            threshold_high: config.threshold_high,
            threshold_medium: config.threshold_medium,
            threshold_low: config.threshold_low,
            reframe_message_template: config.reframe_message_template.clone(),
            refuse_message_template: config.refuse_message_template.clone(),
            comparison_words: if config.comparison_words.is_empty() {
                Settings::default().comparison_words
            } else {
                config.comparison_words.clone()
            },
            domain_words: config
                .domain_words
                .iter()
                .filter(|word| !word.is_empty())
                .map(|word| word.trim().to_lowercase())
                .collect(),
        })
    }

    pub(super) const fn classify_ambiguous(_text: &str, _token: &str) -> (Meaning, f64) {
        let confidence = 0.5_f64;
        (Meaning::OtherMeaning, confidence)
    }

    #[must_use]
    pub fn run(&self, text: &str) -> CompetitorIntentResult {
        let normalized = normalize(text);
        self.evaluate(&normalized, Self::classify_ambiguous)
    }

    #[must_use]
    pub fn run_with<F>(&self, text: &str, classify: F) -> CompetitorIntentResult
    where
        F: Fn(&str, &str) -> (Meaning, f64),
    {
        let normalized = normalize(text);
        self.evaluate(&normalized, classify)
    }

    fn evaluate<F>(&self, normalized: &str, classify: F) -> CompetitorIntentResult
    where
        F: Fn(&str, &str) -> (Meaning, f64),
    {
        let (entities, evidence) = self.brand_entities(normalized);
        let mut result = CompetitorIntentResult {
            intent: Some("other".to_owned()),
            confidence: Some(0.0_f64),
            entities,
            evidence,
            signals: Vec::new(),
            action_hint: Some("allow".to_owned()),
        };
        let matches = find_matches(self, normalized);
        if matches.is_empty() {
            self.category_or_other(normalized, &mut result);
            return result;
        }
        let (mut competitors, match_evidence) = resolve_matches(normalized, matches, classify);
        result.evidence.extend(match_evidence);
        if competitors.is_empty() {
            result.signals.push("other_meaning_or_ambiguous".to_owned());
            return result;
        }
        competitors.sort();
        competitors.dedup();
        result
            .entities
            .insert("competitors".to_owned(), competitors);
        self.classify_resolved(normalized, &mut result);
        result
    }

    fn classify_resolved(&self, normalized: &str, result: &mut CompetitorIntentResult) {
        let has_comparison = has_words(normalized, &self.comparison_words);
        let confidence: f64 = if has_comparison { 0.75 } else { 0.55 };
        let intent = intent_for_confidence(
            confidence,
            self.threshold_high,
            self.threshold_medium,
            self.threshold_low,
        );
        if has_comparison {
            result.evidence.push(CompetitorIntentEvidenceEntry {
                r#type: Some("signal".to_owned()),
                key: Some("comparison".to_owned()),
                r#match: Some("comparison language".to_owned()),
                value: None,
            });
        }
        let action_hint = match intent {
            "log_only" => "log_only",
            "other" => "allow",
            _ => self.policy.get(intent).map_or("allow", String::as_str),
        };
        result.intent = Some(intent.to_owned());
        result.confidence = Some(confidence);
        result.action_hint = Some(action_hint.to_owned());
        result.signals.push("competitor_resolved".to_owned());
        if has_comparison {
            result.signals.push("comparison".to_owned());
        }
    }

    fn brand_entities(
        &self,
        normalized: &str,
    ) -> (
        BTreeMap<String, Vec<String>>,
        Vec<CompetitorIntentEvidenceEntry>,
    ) {
        let mut entities = BTreeMap::from([
            ("brand_self".to_owned(), Vec::new()),
            ("competitors".to_owned(), Vec::new()),
            ("category".to_owned(), Vec::new()),
        ]);
        let mut evidence = Vec::new();
        for brand in &self.brand_self {
            if word_boundary_match(normalized, brand) {
                entities
                    .entry("brand_self".to_owned())
                    .or_default()
                    .push(brand.clone());
                evidence.push(CompetitorIntentEvidenceEntry {
                    r#type: Some("entity".to_owned()),
                    key: Some("brand_self".to_owned()),
                    r#match: Some(brand.clone()),
                    value: Some(brand.clone()),
                });
            }
        }
        (entities, evidence)
    }

    fn category_or_other(&self, text: &str, result: &mut CompetitorIntentResult) {
        if !has_words(text, &self.comparison_words) || !has_words(text, &self.domain_words) {
            return;
        }
        result.evidence.push(CompetitorIntentEvidenceEntry {
            r#type: Some("signal".to_owned()),
            key: Some("category_ranking".to_owned()),
            r#match: Some("comparison + domain".to_owned()),
            value: None,
        });
        result.intent = Some("category_ranking".to_owned());
        result.confidence = Some(0.65_f64);
        result.signals.push("category_ranking".to_owned());
        result.action_hint = Some(
            self.policy
                .get("category_ranking")
                .map_or("reframe", String::as_str)
                .to_owned(),
        );
    }
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
