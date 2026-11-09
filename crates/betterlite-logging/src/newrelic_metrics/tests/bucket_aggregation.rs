use rstest::rstest;
use rust_decimal::Decimal;

use super::support::{
    attribute_at, attributes, decimal_count, envelope, float_count, metric, metric_names, record,
    summary,
};
use crate::newrelic_metrics::records::NewRelicMetricRecord;
use crate::newrelic_metrics::wire::{
    METRIC_COMPLETION_TOKENS, METRIC_COST_USD, METRIC_PROMPT_TOKENS, METRIC_REQUEST_DURATION_MS,
    METRIC_REQUESTS, METRIC_TOTAL_TOKENS,
};

const WINDOW_START_MS: u64 = 1_700_000_000_000;
const WINDOW_END_MS: u64 = 1_700_000_001_500;

fn record_without_budget() -> NewRelicMetricRecord {
    let mut record = record();
    record.team_max_budget = None;
    record.team_spend = None;
    record
}

#[test]
fn a_single_record_bucket_emits_one_metric_per_counter_and_a_summary() {
    let envelope = envelope(&[record_without_budget()], WINDOW_START_MS, WINDOW_END_MS);
    assert_eq!(
        metric_names(&envelope),
        [
            METRIC_REQUESTS,
            METRIC_COST_USD,
            METRIC_PROMPT_TOKENS,
            METRIC_COMPLETION_TOKENS,
            METRIC_TOTAL_TOKENS,
            METRIC_REQUEST_DURATION_MS,
        ]
    );
    assert_eq!(
        float_count(&envelope, METRIC_REQUESTS).to_bits(),
        1.0_f64.to_bits()
    );
    assert_eq!(
        decimal_count(&envelope, METRIC_COST_USD),
        Decimal::new(5, 1)
    );
    assert_eq!(
        float_count(&envelope, METRIC_PROMPT_TOKENS).to_bits(),
        2.0_f64.to_bits()
    );
    assert_eq!(
        float_count(&envelope, METRIC_COMPLETION_TOKENS).to_bits(),
        3.0_f64.to_bits()
    );
    assert_eq!(
        float_count(&envelope, METRIC_TOTAL_TOKENS).to_bits(),
        5.0_f64.to_bits()
    );
    let durations = summary(metric(&envelope, METRIC_REQUEST_DURATION_MS));
    assert_eq!(durations.count, 1);
    assert_eq!(durations.sum.to_bits(), 500.0_f64.to_bits());
    assert_eq!(durations.min.to_bits(), 500.0_f64.to_bits());
    assert_eq!(durations.max.to_bits(), 500.0_f64.to_bits());
}

#[test]
fn counts_and_sums_cover_every_record_in_the_bucket() {
    let mut first = record_without_budget();
    first.response_cost = Decimal::new(5, 1);
    let mut second = record_without_budget();
    second.response_cost = Decimal::new(25, 2);
    second.prompt_tokens = 4;
    second.completion_tokens = 6;
    second.total_tokens = 10;
    second.duration_ms = 750.0_f64;
    let envelope = envelope(&[first, second], WINDOW_START_MS, WINDOW_END_MS);
    assert_eq!(
        float_count(&envelope, METRIC_REQUESTS).to_bits(),
        2.0_f64.to_bits()
    );
    assert_eq!(
        decimal_count(&envelope, METRIC_COST_USD),
        Decimal::new(75, 2)
    );
    assert_eq!(
        float_count(&envelope, METRIC_PROMPT_TOKENS).to_bits(),
        6.0_f64.to_bits()
    );
    assert_eq!(
        float_count(&envelope, METRIC_COMPLETION_TOKENS).to_bits(),
        9.0_f64.to_bits()
    );
    assert_eq!(
        float_count(&envelope, METRIC_TOTAL_TOKENS).to_bits(),
        15.0_f64.to_bits()
    );
    let durations = summary(metric(&envelope, METRIC_REQUEST_DURATION_MS));
    assert_eq!(durations.count, 2);
    assert_eq!(durations.sum.to_bits(), 1250.0_f64.to_bits());
    assert_eq!(durations.min.to_bits(), 500.0_f64.to_bits());
    assert_eq!(durations.max.to_bits(), 750.0_f64.to_bits());
}

#[test]
fn separate_buckets_emit_separate_metric_groups() {
    let mut first = record_without_budget();
    first.model = "gpt-4o".to_owned();
    let mut second = record_without_budget();
    second.model = "gpt-4o-mini".to_owned();
    let envelope = envelope(&[first, second], WINDOW_START_MS, WINDOW_END_MS);
    assert_eq!(metric_names(&envelope).len(), 12);
    assert_eq!(attribute_at(&envelope, 0, "model"), "gpt-4o");
    assert_eq!(attribute_at(&envelope, 6, "model"), "gpt-4o-mini");
}

#[rstest]
#[case::team_alias("team_alias", "Team One", "Team Two")]
#[case::model_group("model_group", "production", "staging")]
#[case::custom_llm_provider("custom_llm_provider", "openai", "anthropic")]
#[case::status("status", "success", "failure")]
fn every_bucket_key_field_separates_records(
    #[case] field: &str,
    #[case] first_value: &str,
    #[case] second_value: &str,
) {
    let first = record_without_budget();
    let mut second = record_without_budget();
    match field {
        "team_alias" => second.team_alias = second_value.to_owned(),
        "model_group" => second.model_group = second_value.to_owned(),
        "custom_llm_provider" => second.custom_llm_provider = second_value.to_owned(),
        "status" => second.status = second_value.to_owned(),
        _ => panic!("an unknown bucket key field"),
    }
    let envelope = envelope(&[first, second], WINDOW_START_MS, WINDOW_END_MS);
    assert_eq!(metric_names(&envelope).len(), 12);
    assert_eq!(attribute_at(&envelope, 0, field), first_value);
    assert_eq!(attribute_at(&envelope, 6, field), second_value);
}

#[test]
fn bucket_attributes_carry_the_first_record_fields() {
    let envelope = envelope(&[record_without_budget()], WINDOW_START_MS, WINDOW_END_MS);
    let fields = attributes(metric(&envelope, METRIC_REQUESTS));
    assert_eq!(fields.get("team_id").map(String::as_str), Some("team-1"));
    assert_eq!(
        fields.get("team_alias").map(String::as_str),
        Some("Team One")
    );
    assert_eq!(
        fields.get("model_group").map(String::as_str),
        Some("production")
    );
    assert_eq!(fields.get("model").map(String::as_str), Some("gpt-4o"));
    assert_eq!(
        fields.get("custom_llm_provider").map(String::as_str),
        Some("openai")
    );
    assert_eq!(fields.get("status").map(String::as_str), Some("success"));
}
