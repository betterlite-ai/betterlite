use rstest::rstest;

use super::support::{attributes, envelope, metric, record};
use crate::newrelic_metrics::wire::{ATTRIBUTE_MAX_LEN, METRIC_REQUESTS};

const WINDOW_START_MS: u64 = 1_700_000_000_000;
const WINDOW_END_MS: u64 = 1_700_000_001_500;

#[rstest]
#[case::ascii("m")]
#[case::multibyte("\u{e9}")]
fn attribute_values_are_truncated_to_the_metric_api_limit(#[case] unit: &str) {
    let mut record = record();
    record.model = unit.repeat(300);
    record.team_max_budget = None;
    let envelope = envelope(&[record], WINDOW_START_MS, WINDOW_END_MS);
    let model = attributes(metric(&envelope, METRIC_REQUESTS))
        .get("model")
        .expect("a model attribute");
    assert_eq!(model.chars().count(), ATTRIBUTE_MAX_LEN);
}

#[test]
fn attribute_values_at_the_limit_are_kept_whole() {
    let mut record = record();
    record.model_group = "g".repeat(ATTRIBUTE_MAX_LEN);
    record.team_max_budget = None;
    let envelope = envelope(&[record], WINDOW_START_MS, WINDOW_END_MS);
    let model_group = attributes(metric(&envelope, METRIC_REQUESTS))
        .get("model_group")
        .expect("a model_group attribute");
    assert_eq!(model_group.chars().count(), ATTRIBUTE_MAX_LEN);
}

#[test]
fn empty_attribute_values_are_omitted() {
    let mut record = record();
    record.team_id = String::new();
    record.team_alias = String::new();
    record.model_group = String::new();
    record.model = String::new();
    record.custom_llm_provider = String::new();
    record.team_max_budget = None;
    let envelope = envelope(&[record], WINDOW_START_MS, WINDOW_END_MS);
    let fields = attributes(metric(&envelope, METRIC_REQUESTS));
    assert_eq!(fields.len(), 1);
    assert_eq!(fields.get("status").map(String::as_str), Some("success"));
}

#[test]
fn only_empty_attribute_values_are_omitted() {
    let mut record = record();
    record.team_alias = String::new();
    let envelope = envelope(&[record], WINDOW_START_MS, WINDOW_END_MS);
    let fields = attributes(metric(&envelope, METRIC_REQUESTS));
    assert!(!fields.contains_key("team_alias"));
    assert_eq!(fields.get("team_id").map(String::as_str), Some("team-1"));
}
