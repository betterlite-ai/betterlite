use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::guardrail_usage_rollups::{
    GuardrailCounts, GuardrailSpendLog, MetricsKey, UsageUnitIncrement, rollups,
};

use super::support::{UnitCall, capture_logs, from_json, metrics_key, unit_key};

fn payload_with_entries(entries: &str) -> GuardrailSpendLog {
    from_json(&format!(
        r#"{{"request_id": "req-1", "startTime": "2026-01-02T03:04:05Z", "team_id": "team-1", "api_key": "sk-1", "metadata": {{"guardrail_information": [{entries}]}}}}"#
    ))
}

fn counts(requests: i64, passed: i64, blocked: i64, flagged: i64) -> GuardrailCounts {
    GuardrailCounts {
        requests_evaluated: requests,
        passed_count: passed,
        blocked_count: blocked,
        flagged_count: flagged,
    }
}

fn increment(units: i64, cost: Decimal, untracked_units: i64) -> UsageUnitIncrement {
    UsageUnitIncrement {
        units,
        cost,
        untracked_units,
    }
}

fn unit_call(guardrail_id: &str, usage_unit: &str, increment: UsageUnitIncrement) -> UnitCall {
    UnitCall::new(&unit_key(guardrail_id, usage_unit), increment)
}

fn metrics_of(payloads: &[GuardrailSpendLog]) -> Vec<(MetricsKey, GuardrailCounts)> {
    rollups::fresh_metrics(payloads).into_iter().collect()
}

fn units_of(payloads: &[GuardrailSpendLog]) -> Vec<UnitCall> {
    rollups::fresh_units(payloads)
        .into_iter()
        .map(|(key, increment)| UnitCall::new(&key, increment))
        .collect()
}

#[test]
fn counts_requests_and_actions_per_guardrail_and_date() {
    let payload = payload_with_entries(
        r#"{"guardrail_id": "g1", "guardrail_status": "success"},
           {"guardrail_id": "g1", "guardrail_status": "flagged"},
           {"guardrail_id": "g1", "guardrail_status": "not_run"},
           {"guardrail_id": "g2", "guardrail_status": "blocked"}"#,
    );
    assert_eq!(
        metrics_of(&[payload]),
        [
            (metrics_key("g1"), counts(2, 1, 0, 1)),
            (metrics_key("g2"), counts(1, 0, 1, 0)),
        ]
    );
}

#[test]
fn counts_each_entry_of_a_repeated_request_separately() {
    let payload = payload_with_entries(r#"{"guardrail_id": "g1", "guardrail_status": "success"}"#);
    assert_eq!(
        metrics_of(&[payload.clone(), payload]),
        [(metrics_key("g1"), counts(2, 2, 0, 0))]
    );
}

#[rstest]
#[case::no_id_or_name(r#"{"guardrail_status": "success"}"#)]
#[case::empty_id_and_name(
    r#"{"guardrail_id": "", "guardrail_name": "", "guardrail_status": "success"}"#
)]
fn skips_an_entry_without_a_guardrail_id_or_name(#[case] entry: &str) {
    assert!(metrics_of(&[payload_with_entries(entry)]).is_empty());
}

#[test]
fn sums_units_per_unit_name_with_the_priced_and_unpriced_parts() {
    let payload = payload_with_entries(
        r#"{"guardrail_id": "g1", "guardrail_usage": {"a": 2, "b": 3}, "guardrail_cost_by_unit": {"a": 0.5}}"#,
    );
    assert_eq!(
        units_of(&[payload]),
        [
            unit_call("g1", "a", increment(2, Decimal::new(5, 1), 0)),
            unit_call("g1", "b", increment(3, Decimal::ZERO, 3)),
        ]
    );
}

#[test]
fn sums_two_entries_of_the_same_unit_key() {
    let payload = payload_with_entries(
        r#"{"guardrail_id": "g1", "guardrail_usage": {"a": 2}},
           {"guardrail_id": "g1", "guardrail_usage": {"a": 2}, "guardrail_cost_by_unit": {"a": 0.5}}"#,
    );
    assert_eq!(
        units_of(&[payload]),
        [unit_call("g1", "a", increment(4, Decimal::new(5, 1), 2))]
    );
}

#[rstest]
#[case::report_only(r#"{"guardrail_id": "g1", "guardrail_usage": {"a": 1}, "guardrail_cost_by_unit": {"a": 0.5}, "guardrail_cost_in_spend": false}"#)]
#[case::report_only_text_flag(r#"{"guardrail_id": "g1", "guardrail_usage": {"a": 1}, "guardrail_cost_by_unit": {"a": 0.5}, "guardrail_cost_in_spend": "false"}"#)]
#[case::report_only_zero_flag(r#"{"guardrail_id": "g1", "guardrail_usage": {"a": 1}, "guardrail_cost_by_unit": {"a": 0.5}, "guardrail_cost_in_spend": 0}"#)]
#[case::negative_cost(
    r#"{"guardrail_id": "g1", "guardrail_usage": {"a": 1}, "guardrail_cost_by_unit": {"a": -0.5}}"#
)]
#[case::negative_sibling(r#"{"guardrail_id": "g1", "guardrail_usage": {"a": 1}, "guardrail_cost_by_unit": {"a": 0.5, "b": -1}}"#)]
fn treats_the_whole_entry_as_unpriced(#[case] entry: &str) {
    assert_eq!(
        units_of(&[payload_with_entries(entry)]),
        [unit_call("g1", "a", increment(1, Decimal::ZERO, 1))]
    );
}

#[test]
fn keeps_a_zero_cost_unit_priced() {
    let entry = r#"{"guardrail_id": "g1", "guardrail_usage": {"a": 1}, "guardrail_cost_by_unit": {"a": 0}}"#;
    assert_eq!(
        units_of(&[payload_with_entries(entry)]),
        [unit_call("g1", "a", increment(1, Decimal::ZERO, 0))]
    );
}

#[test]
fn prices_an_entry_whose_in_spend_flag_is_null() {
    let entry = r#"{"guardrail_id": "g1", "guardrail_usage": {"a": 1}, "guardrail_cost_by_unit": {"a": 0.5}, "guardrail_cost_in_spend": null}"#;
    assert_eq!(
        units_of(&[payload_with_entries(entry)]),
        [unit_call("g1", "a", increment(1, Decimal::new(5, 1), 0))]
    );
}

#[test]
fn leaves_a_unit_with_a_null_price_unpriced() {
    let entry = r#"{"guardrail_id": "g1", "guardrail_usage": {"a": 2, "b": 3}, "guardrail_cost_by_unit": {"a": 0.5, "b": null}}"#;
    assert_eq!(
        units_of(&[payload_with_entries(entry)]),
        [
            unit_call("g1", "a", increment(2, Decimal::new(5, 1), 0)),
            unit_call("g1", "b", increment(3, Decimal::ZERO, 3)),
        ]
    );
}

// the shared pricing rejects a negative-signed zero that LiteLLM's Field(ge=0) reads as zero, as docs/deviations/B3688.yaml records
#[test]
fn treats_a_negative_signed_zero_price_as_unpriced() {
    let entry = r#"{"guardrail_id": "g1", "guardrail_usage": {"a": 1}, "guardrail_cost_by_unit": {"a": -0.0}}"#;
    assert_eq!(
        units_of(&[payload_with_entries(entry)]),
        [unit_call("g1", "a", increment(1, Decimal::ZERO, 1))]
    );
}

#[rstest]
#[case::text_price(r#""0.5""#, Decimal::new(5, 1))]
#[case::boolean_price("true", Decimal::new(1, 0))]
fn prices_a_cost_the_shared_pricing_module_coerces(#[case] price: &str, #[case] expected: Decimal) {
    let entry = format!(
        r#"{{"guardrail_id": "g1", "guardrail_usage": {{"a": 2}}, "guardrail_cost_by_unit": {{"a": {price}}}}}"#
    );
    assert_eq!(
        units_of(&[payload_with_entries(&entry)]),
        [unit_call("g1", "a", increment(2, expected, 0))]
    );
}

#[rstest]
#[case::price_map_is_a_number("2.5")]
#[case::price_map_is_a_string(r#""0.5""#)]
#[case::price_map_is_a_list("[0.5]")]
fn treats_a_cost_map_that_is_not_an_object_as_unpriced(#[case] price_map: &str) {
    let entry = format!(
        r#"{{"guardrail_id": "g1", "guardrail_usage": {{"a": 1}}, "guardrail_cost_by_unit": {price_map}}}"#
    );
    assert_eq!(
        units_of(&[payload_with_entries(&entry)]),
        [unit_call("g1", "a", increment(1, Decimal::ZERO, 1))]
    );
}

#[test]
fn treats_a_price_the_shared_pricing_module_cannot_represent_as_unpriced_with_a_warning() {
    let capture = capture_logs();
    let entry = r#"{"guardrail_id": "g1", "guardrail_usage": {"a": 1}, "guardrail_cost_by_unit": {"a": 1e400}}"#;
    assert_eq!(
        units_of(&[payload_with_entries(entry)]),
        [unit_call("g1", "a", increment(1, Decimal::ZERO, 1))]
    );
    assert!(capture.logs.has("WARN", "no finite price"));
}

#[rstest]
#[case::whole("2", Some(2))]
#[case::zero("0", None)]
#[case::negative("-3", None)]
#[case::fraction("1.5", None)]
#[case::truth("true", None)]
#[case::text(r#""2""#, None)]
#[case::null("null", None)]
#[case::list("[2]", None)]
#[case::object(r#"{"count": 2}"#, None)]
fn skips_a_unit_count_that_is_not_a_positive_integer(
    #[case] value: &str,
    #[case] expected: Option<i64>,
) {
    let payload = from_json(&format!(
        r#"{{"request_id": "req-1", "startTime": "2026-01-02T03:04:05Z", "metadata": {{"guardrail_information": [{{"guardrail_id": "g1", "guardrail_usage": {{"a": {value}}}}}]}}}}"#
    ));
    let units = units_of(&[payload]);
    assert_eq!(units.first().map(|call| call.units), expected);
}

#[test]
fn keys_a_unit_by_the_entry_name_when_the_name_to_id_map_does_not_apply() {
    let payload = payload_with_entries(
        r#"{"guardrail_name": "pii", "guardrail_id": "g1", "guardrail_usage": {"a": 1}},
           {"guardrail_name": "pii", "guardrail_usage": {"a": 2}}"#,
    );
    assert_eq!(
        units_of(&[payload]),
        [
            unit_call("g1", "a", increment(1, Decimal::ZERO, 1)),
            unit_call("pii", "a", increment(2, Decimal::ZERO, 2)),
        ]
    );
}

#[test]
fn falls_back_to_the_name_when_the_guardrail_id_is_empty() {
    let payload = payload_with_entries(
        r#"{"guardrail_name": "pii", "guardrail_id": "", "guardrail_usage": {"b": 4}}"#,
    );
    assert_eq!(
        units_of(&[payload]),
        [unit_call("pii", "b", increment(4, Decimal::ZERO, 4))]
    );
}

#[test]
fn leaves_the_api_key_out_of_the_unit_key_diagnostic() {
    let text = format!("{:?}", unit_key("g1", "a"));
    assert!(text.contains("UsageUnitKey"));
    assert!(text.contains("usage_unit"));
    assert!(!text.contains("sk-1"));
}

#[test]
fn keeps_the_queued_rows_first_when_merging_counters() {
    let base = IndexMap::from([
        (metrics_key("g1"), counts(1, 1, 0, 0)),
        (metrics_key("g2"), counts(1, 0, 1, 0)),
    ]);
    let extra = IndexMap::from([
        (metrics_key("g2"), counts(2, 0, 0, 2)),
        (metrics_key("g3"), counts(1, 1, 0, 0)),
    ]);
    let merged = rollups::merged_rows(base, extra, rollups::summed_counts);
    assert_eq!(
        merged.into_iter().collect::<Vec<_>>(),
        [
            (metrics_key("g1"), counts(1, 1, 0, 0)),
            (metrics_key("g2"), counts(3, 0, 1, 2)),
            (metrics_key("g3"), counts(1, 1, 0, 0)),
        ]
    );
}

#[test]
fn sums_the_parts_of_a_merged_unit_increment() {
    let key = unit_key("g1", "a");
    let base = IndexMap::from([(key.clone(), increment(1, Decimal::new(5, 1), 2))]);
    let extra = IndexMap::from([(key.clone(), increment(3, Decimal::new(25, 2), 4))]);
    let merged = rollups::merged_rows(base, extra, rollups::summed_units);
    assert_eq!(
        merged.get(&key),
        Some(&increment(4, Decimal::new(75, 2), 6))
    );
}

fn metric_rows(count: usize) -> IndexMap<MetricsKey, GuardrailCounts> {
    let mut rows = IndexMap::new();
    for index in 0..count {
        let key = metrics_key(&format!("g{index}"));
        rows.insert(key, counts(1, 1, 0, 0));
    }
    rows
}

#[test]
fn keeps_every_pending_row_up_to_the_cap_without_warning() {
    let capture = capture_logs();
    let rows = rollups::capped(metric_rows(10_000), "daily metrics");
    assert_eq!(rows.len(), 10_000);
    assert!(!capture.logs.has("WARN", "exceeds"));
}

#[test]
fn drops_the_oldest_pending_rows_past_the_cap_with_a_warning() {
    let capture = capture_logs();
    let rows = rollups::capped(metric_rows(10_001), "daily metrics");
    assert_eq!(rows.len(), 10_000);
    assert_eq!(
        rows.first().map(|(key, _)| key.guardrail_id.as_str()),
        Some("g1")
    );
    assert!(capture.logs.has("WARN", "exceeds 10000 rows"));
}
