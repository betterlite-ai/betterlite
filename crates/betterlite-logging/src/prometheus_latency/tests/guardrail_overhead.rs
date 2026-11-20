#![cfg(test)]

use rstest::rstest;
use rust_decimal::Decimal;

use crate::fields::payload::records::GuardrailInformation;
use crate::prometheus_latency::guardrail_overhead::{
    GuardrailRecords, OverheadObservation, guardrail_is_additive, guardrail_overhead_seconds,
    set_guardrail_overhead_metric,
};
use crate::prometheus_latency::labels::LabelInputs;
use crate::prometheus_latency::latency_histograms::MetricHandle;
use crate::prometheus_latency::test_support::{
    EventLog, Recorded, RecordingMetric, dec, end_user_settings, label_settings, label_values,
    observed, recorded, supported,
};

fn record(text: &str) -> GuardrailInformation {
    serde_json::from_str(text).unwrap()
}

#[rstest]
#[case::pre_call(r#"{"guardrail_mode": "pre_call"}"#, true)]
#[case::post_call(r#"{"guardrail_mode": "post_call"}"#, true)]
#[case::both_additive_modes(r#"{"guardrail_mode": ["pre_call", "post_call"]}"#, true)]
#[case::pre_call_with_a_mode_object(r#"{"guardrail_mode": ["pre_call", {"tags": ["pii"]}]}"#, true)]
#[case::during_call(r#"{"guardrail_mode": "during_call"}"#, false)]
#[case::logging_only(r#"{"guardrail_mode": "logging_only"}"#, false)]
#[case::pre_mcp_call(r#"{"guardrail_mode": "pre_mcp_call"}"#, false)]
#[case::mixed_list(r#"{"guardrail_mode": ["pre_call", "during_call"]}"#, false)]
#[case::object_only(r#"{"guardrail_mode": {"tags": ["pii"]}}"#, false)]
#[case::object_only_in_a_list(r#"{"guardrail_mode": [{"tags": ["pii"]}]}"#, false)]
#[case::no_mode("{}", false)]
#[case::null_mode(r#"{"guardrail_mode": null}"#, false)]
#[case::number_mode(r#"{"guardrail_mode": 3}"#, false)]
#[case::empty_list(r#"{"guardrail_mode": []}"#, false)]
fn guardrail_additivity_follows_the_mode_values(#[case] text: &str, #[case] additive: bool) {
    assert_eq!(guardrail_is_additive(&record(text)), additive);
}

#[test]
fn guardrail_overhead_sums_only_additive_durations() {
    let records = GuardrailRecords::Multiple(vec![
        record(r#"{"guardrail_mode": "pre_call", "duration": 0.5}"#),
        record(r#"{"guardrail_mode": "during_call", "duration": 10}"#),
        record(r#"{"guardrail_mode": ["post_call"], "duration": 0.25}"#),
        record(r#"{"guardrail_mode": "post_call"}"#),
    ]);
    assert_eq!(guardrail_overhead_seconds(&records), Some(dec(75, 2)));
}

#[test]
fn a_single_record_counts_as_a_list_of_one() {
    let records =
        GuardrailRecords::Single(record(r#"{"guardrail_mode": "pre_call", "duration": 1.5}"#));
    assert_eq!(guardrail_overhead_seconds(&records), Some(dec(15, 1)));
}

#[test]
fn no_guardrail_records_sum_to_zero() {
    assert_eq!(
        guardrail_overhead_seconds(&GuardrailRecords::Absent),
        Some(Decimal::ZERO)
    );
    assert_eq!(
        guardrail_overhead_seconds(&GuardrailRecords::Multiple(Vec::new())),
        Some(Decimal::ZERO)
    );
}

fn overhead_events(overhead_ms: Option<Decimal>, records: &GuardrailRecords) -> Vec<Recorded> {
    let events = EventLog::default();
    let metric = RecordingMetric::with_log("overhead", &events);
    let supported_labels = supported(&["model"]);
    let histogram = MetricHandle {
        metric: &metric,
        supported_labels: &supported_labels,
    };
    let settings = label_settings();
    let values = label_values(None, None);
    let inputs = LabelInputs {
        values: &values,
        settings: &settings,
        end_user_settings: end_user_settings(),
        context: None,
    };
    let observation = OverheadObservation {
        overhead_ms,
        guardrail_records: records,
    };
    set_guardrail_overhead_metric(&histogram, &observation, &inputs).unwrap();
    recorded(&events)
}

#[test]
fn overhead_metric_adds_guardrail_seconds_to_the_millisecond_overhead() {
    let records =
        GuardrailRecords::Single(record(r#"{"guardrail_mode": "pre_call", "duration": 0.5}"#));
    assert_eq!(
        overhead_events(Some(dec(500, 0)), &records),
        vec![observed("overhead", &[("model", "gpt-4o")], dec(10, 1))]
    );
}

#[test]
fn guardrail_time_records_without_an_overhead() {
    let records = GuardrailRecords::Single(record(
        r#"{"guardrail_mode": "post_call", "duration": 0.5}"#,
    ));
    assert_eq!(
        overhead_events(None, &records),
        vec![observed("overhead", &[("model", "gpt-4o")], dec(5, 1))]
    );
}

#[test]
fn a_zero_overhead_still_records_with_guardrail_time() {
    let records = GuardrailRecords::Single(record(
        r#"{"guardrail_mode": "post_call", "duration": 0.5}"#,
    ));
    assert_eq!(
        overhead_events(Some(Decimal::ZERO), &records),
        vec![observed("overhead", &[("model", "gpt-4o")], dec(5, 1))]
    );
}

#[test]
fn a_zero_overhead_without_guardrail_time_still_records() {
    assert_eq!(
        overhead_events(Some(Decimal::ZERO), &GuardrailRecords::Absent),
        vec![observed("overhead", &[("model", "gpt-4o")], dec(0, 0))]
    );
}

#[test]
fn a_missing_overhead_without_guardrail_time_records_nothing() {
    assert_eq!(overhead_events(None, &GuardrailRecords::Absent), Vec::new());
}

#[test]
fn non_additive_guardrail_time_is_not_recorded_on_its_own() {
    let records = GuardrailRecords::Single(record(
        r#"{"guardrail_mode": "during_call", "duration": 5}"#,
    ));
    assert_eq!(overhead_events(None, &records), Vec::new());
}
