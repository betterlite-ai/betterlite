#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::content_filter_records::overrides::CompetitorIntentEvidenceEntry;
use betterlite_config::sections::content_filter_records::{
    BlockedWordDetection, CategoryKeywordDetection, CompetitorIntentDetection,
    ContentFilterDetection, PatternDetection,
};

use crate::content_filter_streaming::merge::merge_detections;

fn blocked_word(keyword: &str, description: Option<&str>) -> ContentFilterDetection {
    ContentFilterDetection::BlockedWord(BlockedWordDetection {
        r#type: "blocked_word".to_owned(),
        keyword: keyword.to_owned(),
        action: "MASK".to_owned(),
        description: description.map(str::to_owned),
    })
}

fn pattern(name: &str) -> ContentFilterDetection {
    ContentFilterDetection::Pattern(PatternDetection {
        r#type: "pattern".to_owned(),
        pattern_name: name.to_owned(),
        action: "MASK".to_owned(),
    })
}

fn category_keyword(keyword: &str, severity: &str) -> ContentFilterDetection {
    ContentFilterDetection::CategoryKeyword(CategoryKeywordDetection {
        r#type: "category_keyword".to_owned(),
        category: "stream".to_owned(),
        keyword: keyword.to_owned(),
        severity: severity.to_owned(),
        action: "MASK".to_owned(),
    })
}

fn competitor(intent: &str, confidence: f64, signals: &[&str]) -> ContentFilterDetection {
    ContentFilterDetection::CompetitorIntent(CompetitorIntentDetection {
        r#type: "competitor_intent".to_owned(),
        intent: intent.to_owned(),
        confidence,
        action_hint: "BLOCK".to_owned(),
        entities: BTreeMap::new(),
        signals: signals.iter().map(|signal| (*signal).to_owned()).collect(),
        evidence: vec![CompetitorIntentEvidenceEntry {
            r#type: Some("keyword".to_owned()),
            key: Some("signal".to_owned()),
            value: Some("matched".to_owned()),
            r#match: Some("match".to_owned()),
        }],
    })
}

fn competitor_evidence(entries: &[(&str, &str, &str, &str)]) -> ContentFilterDetection {
    let evidence = entries
        .iter()
        .map(
            |(entry_type, entry_key, entry_value, entry_match)| CompetitorIntentEvidenceEntry {
                r#type: Some((*entry_type).to_owned()),
                key: Some((*entry_key).to_owned()),
                value: Some((*entry_value).to_owned()),
                r#match: Some((*entry_match).to_owned()),
            },
        )
        .collect();
    ContentFilterDetection::CompetitorIntent(CompetitorIntentDetection {
        r#type: "competitor_intent".to_owned(),
        intent: "airline".to_owned(),
        confidence: 0.9,
        action_hint: "BLOCK".to_owned(),
        entities: BTreeMap::new(),
        signals: vec!["a".to_owned()],
        evidence,
    })
}

#[test]
fn merge_keeps_the_first_of_each_equal_detection() {
    let merged = merge_detections(&[
        blocked_word("secret", None),
        blocked_word("secret", None),
        blocked_word("secret", None),
    ]);
    assert_eq!(merged.len(), 1);
    assert_eq!(
        serde_json::to_value(&merged[0]).unwrap(),
        serde_json::to_value(blocked_word("secret", None)).unwrap()
    );
}

#[test]
fn merge_keeps_detections_that_differ_by_value() {
    let merged = merge_detections(&[
        blocked_word("secret", None),
        blocked_word("token", None),
        blocked_word("secret", Some("policy")),
    ]);
    assert_eq!(merged.len(), 3);
}

#[test]
fn merge_keeps_the_first_occurrence_order() {
    let merged = merge_detections(&[
        blocked_word("secret", None),
        pattern("peek"),
        blocked_word("secret", None),
    ]);
    assert_eq!(merged.len(), 2);
    assert_eq!(
        serde_json::to_value(&merged[0]).unwrap(),
        serde_json::to_value(blocked_word("secret", None)).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&merged[1]).unwrap(),
        serde_json::to_value(pattern("peek")).unwrap()
    );
}

#[test]
fn merge_keeps_the_first_of_each_equal_pattern() {
    let merged = merge_detections(&[pattern("peek"), pattern("peek")]);
    assert_eq!(merged.len(), 1);
    assert_eq!(
        serde_json::to_value(&merged[0]).unwrap(),
        serde_json::to_value(pattern("peek")).unwrap()
    );
}

#[test]
fn merge_compares_pattern_name_and_action() {
    let other_action = ContentFilterDetection::Pattern(PatternDetection {
        r#type: "pattern".to_owned(),
        pattern_name: "peek".to_owned(),
        action: "BLOCK".to_owned(),
    });
    let merged = merge_detections(&[pattern("peek"), pattern("other"), other_action]);
    assert_eq!(merged.len(), 3);
}

#[test]
fn merge_compares_category_keyword_severity() {
    let merged = merge_detections(&[
        category_keyword("token", "high"),
        category_keyword("token", "high"),
        category_keyword("token", "low"),
    ]);
    assert_eq!(merged.len(), 2);
}

#[test]
fn merge_compares_competitor_intent_fields() {
    let merged = merge_detections(&[
        competitor("airline", 0.9, &["a"]),
        competitor("airline", 0.9, &["a"]),
        competitor("airline", 0.8, &["a"]),
        competitor("airline", 0.9, &["b"]),
        competitor("other", 0.9, &["a"]),
    ]);
    assert_eq!(merged.len(), 4);
}

#[test]
fn merge_compares_competitor_intent_evidence() {
    let merged = merge_detections(&[
        competitor_evidence(&[("keyword", "signal", "matched", "match")]),
        competitor_evidence(&[("keyword", "signal", "matched", "match")]),
        competitor_evidence(&[("phrase", "signal", "matched", "match")]),
        competitor_evidence(&[("keyword", "other", "matched", "match")]),
        competitor_evidence(&[("keyword", "signal", "other", "match")]),
        competitor_evidence(&[("keyword", "signal", "matched", "other")]),
    ]);
    assert_eq!(merged.len(), 5);
}

#[test]
fn merge_keeps_competitor_evidence_of_different_lengths_apart() {
    let merged = merge_detections(&[
        competitor_evidence(&[("keyword", "signal", "matched", "match")]),
        competitor_evidence(&[
            ("keyword", "signal", "matched", "match"),
            ("phrase", "other", "matched", "match"),
        ]),
    ]);
    assert_eq!(merged.len(), 2);
}

#[test]
fn merge_treats_a_nan_confidence_as_unequal_to_itself() {
    let merged = merge_detections(&[
        competitor("airline", f64::NAN, &["a"]),
        competitor("airline", f64::NAN, &["a"]),
    ]);
    assert_eq!(merged.len(), 2);
}

#[test]
fn merge_keeps_different_detection_variants_apart() {
    let merged = merge_detections(&[blocked_word("token", None), category_keyword("token", " ")]);
    assert_eq!(merged.len(), 2);
}

#[test]
fn merge_of_nothing_is_nothing() {
    assert!(merge_detections(&[]).is_empty());
}
