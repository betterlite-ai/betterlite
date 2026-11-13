#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::shared::{analyzer, item, settings};
use crate::presidio_analysis::PresidioAnalyzer;
use crate::presidio_analysis::filter::filter_by_score;

fn thresholds(entries: &[(&str, f64)]) -> BTreeMap<String, f64> {
    entries
        .iter()
        .map(|&(name, minimum)| (name.to_owned(), minimum))
        .collect()
}

fn kept_types(analyzer: &PresidioAnalyzer) -> Vec<String> {
    let found = vec![
        item("PERSON", 0, 4, 0.4),
        item("PERSON", 5, 9, 0.8),
        item("EMAIL_ADDRESS", 10, 20, 0.6),
        item("PHONE_NUMBER", 21, 30, 0.3),
    ];
    filter_by_score(&analyzer.settings, found)
        .into_iter()
        .map(|kept| format!("{}@{}", kept.entity_type.unwrap(), kept.start.unwrap()))
        .collect()
}

#[test]
fn filter_score_returns_results_unchanged_without_thresholds_or_deny_list() {
    let analyzer = analyzer(settings("http://unused"));
    assert_eq!(
        kept_types(&analyzer),
        vec![
            "PERSON@0",
            "PERSON@5",
            "EMAIL_ADDRESS@10",
            "PHONE_NUMBER@21"
        ]
    );
}

#[rstest]
#[case::per_entity_threshold(&[("PERSON", 0.5_f64)], &["PERSON@5", "EMAIL_ADDRESS@10", "PHONE_NUMBER@21"])]
#[case::all_threshold_applies_to_every_type(&[("ALL", 0.5_f64)], &["PERSON@5", "EMAIL_ADDRESS@10"])]
#[case::specific_threshold_beats_all(&[("ALL", 0.9_f64), ("PERSON", 0.1_f64)], &["PERSON@0", "PERSON@5"])]
#[case::threshold_is_inclusive(&[("EMAIL_ADDRESS", 0.6_f64)], &["PERSON@0", "PERSON@5", "EMAIL_ADDRESS@10", "PHONE_NUMBER@21"])]
fn filter_score_drops_detections_below_the_threshold(
    #[case] configured: &[(&str, f64)],
    #[case] expected: &[&str],
) {
    let mut analyzer_settings = settings("http://unused");
    analyzer_settings.score_thresholds = thresholds(configured);
    let analyzer = analyzer(analyzer_settings);
    assert_eq!(kept_types(&analyzer), expected);
}

#[test]
fn filter_score_drops_denied_entity_types_regardless_of_score() {
    let mut analyzer_settings = settings("http://unused");
    analyzer_settings.entities_deny_list = vec!["PERSON".to_owned()];
    let analyzer = analyzer(analyzer_settings);
    assert_eq!(
        kept_types(&analyzer),
        vec!["EMAIL_ADDRESS@10", "PHONE_NUMBER@21"]
    );
}

#[test]
fn filter_score_applies_the_deny_list_and_thresholds_together() {
    let mut analyzer_settings = settings("http://unused");
    analyzer_settings.entities_deny_list = vec!["EMAIL_ADDRESS".to_owned()];
    analyzer_settings.score_thresholds = thresholds(&[("ALL", 0.35_f64)]);
    let analyzer = analyzer(analyzer_settings);
    assert_eq!(kept_types(&analyzer), vec!["PERSON@0", "PERSON@5"]);
}

#[test]
fn filter_score_drops_a_scoreless_detection_when_a_threshold_applies() {
    let mut analyzer_settings = settings("http://unused");
    analyzer_settings.score_thresholds = thresholds(&[("ALL", 0.1_f64)]);
    let analyzer = analyzer(analyzer_settings);
    let mut scoreless = item("PERSON", 0, 4, 0.0);
    scoreless.score = None;
    assert!(filter_by_score(&analyzer.settings, vec![scoreless]).is_empty());
}

#[test]
fn filter_score_keeps_types_that_have_no_threshold_and_no_all_entry() {
    let mut analyzer_settings = settings("http://unused");
    analyzer_settings.score_thresholds = thresholds(&[("PERSON", 0.99_f64)]);
    let analyzer = analyzer(analyzer_settings);
    assert_eq!(
        kept_types(&analyzer),
        vec!["EMAIL_ADDRESS@10", "PHONE_NUMBER@21"]
    );
}
