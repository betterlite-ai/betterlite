use std::collections::BTreeMap;

use betterlite_config::sections::content_filter_records::overrides::{
    CompetitorIntentEvidenceEntry, CompetitorIntentResult,
};

pub(super) fn empty_entities() -> BTreeMap<String, Vec<String>> {
    let entities = BTreeMap::from([
        ("brand_self".to_owned(), Vec::new()),
        ("competitors".to_owned(), Vec::new()),
        ("category".to_owned(), Vec::new()),
    ]);
    entities
}

pub(super) fn other_result(
    entities: &BTreeMap<String, Vec<String>>,
    evidence: Vec<CompetitorIntentEvidenceEntry>,
) -> CompetitorIntentResult {
    let result = CompetitorIntentResult {
        intent: Some("other".to_owned()),
        confidence: Some(0.0),
        entities: entities.clone(),
        signals: vec!["other_meaning_or_ambiguous".to_owned()],
        action_hint: Some("allow".to_owned()),
        evidence,
    };
    result
}

pub(super) fn entity_evidence(
    key: &str,
    value: &str,
    matched: &str,
) -> CompetitorIntentEvidenceEntry {
    let evidence = CompetitorIntentEvidenceEntry {
        r#type: Some("entity".to_owned()),
        key: Some(key.to_owned()),
        value: Some(value.to_owned()),
        r#match: Some(matched.to_owned()),
    };
    evidence
}

pub(super) fn signal_evidence(key: &str, matched: &str) -> CompetitorIntentEvidenceEntry {
    let evidence = CompetitorIntentEvidenceEntry {
        r#type: Some("signal".to_owned()),
        key: Some(key.to_owned()),
        value: None,
        r#match: Some(matched.to_owned()),
    };
    evidence
}
