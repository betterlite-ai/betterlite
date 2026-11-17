#![cfg(test)]

use rust_decimal::Decimal;
use tracing::Level;

use super::super::super::support;

const SHORT_AUDIO: &str = "https://eastus.stt.speech.microsoft.com/speech/recognition/conversation/cognitiveservices/v1?language=en-US";
const FAST: &str = "https://eastus.api.cognitive.microsoft.com/speechtotext/transcriptions:transcribe?api-version=2024-11-15";
const BATCH: &str = "https://eastus.api.cognitive.microsoft.com/speechtotext/v3.2/transcriptions";
const RECOGNIZED: &[u8] = br#"{"Offset": 5000000, "Duration": 25000000}"#;

#[test]
fn the_derived_seconds_are_priced_from_the_fixed_azure_stt_entry() {
    let logging_obj = support::logging_object();
    let lookup = support::RecordingLookup::priced(r#"{"input_cost_per_second": 0.5}"#);
    let outcome = support::run(&logging_obj, &lookup, SHORT_AUDIO, None, RECOGNIZED);
    assert_eq!(support::cost_of(&outcome), support::dec("1.5"));
    assert_eq!(
        lookup.calls(),
        vec![(
            "azure/speech/azure-stt".to_owned(),
            Some("azure".to_owned())
        )]
    );
}

#[test]
fn a_positive_output_rate_bills_the_completion_side() {
    let logging_obj = support::logging_object();
    let lookup = support::RecordingLookup::priced(
        r#"{"input_cost_per_second": 0.1, "output_cost_per_second": 0.5}"#,
    );
    let outcome = support::run(&logging_obj, &lookup, SHORT_AUDIO, None, RECOGNIZED);
    assert_eq!(support::cost_of(&outcome), support::dec("1.5"));
}

#[test]
fn a_zero_duration_is_billed_without_asking_the_registry() {
    let logging_obj = support::logging_object();
    let lookup = support::RecordingLookup::unpriced();
    let (logs, _guard) = support::capture_logs(Level::WARN);
    let outcome = support::run(&logging_obj, &lookup, SHORT_AUDIO, None, b"{}");
    assert_eq!(support::cost_of(&outcome), Decimal::ZERO);
    assert!(lookup.calls().is_empty());
    assert!(
        logs.text().is_empty(),
        "a zero duration warns about nothing"
    );
}

#[test]
fn a_missing_price_logs_a_warning_and_bills_zero() {
    let logging_obj = support::logging_object();
    let lookup = support::RecordingLookup::unpriced();
    let (logs, _guard) = support::capture_logs(Level::WARN);
    let outcome = support::run(&logging_obj, &lookup, SHORT_AUDIO, None, RECOGNIZED);
    assert_eq!(support::cost_of(&outcome), Decimal::ZERO);
    assert_eq!(
        support::recorded_cost(&logging_obj),
        Some(Decimal::ZERO),
        "the spend row survives a missing price"
    );
    let text = logs.text();
    assert!(text.contains("azure/speech/azure-stt"));
    assert!(text.contains("zero cost"));
}

#[test]
fn a_malformed_price_logs_a_warning_and_bills_zero() {
    let logging_obj = support::logging_object();
    let lookup = support::RecordingLookup::priced(r#"{"input_cost_per_second": []}"#);
    let (logs, _guard) = support::capture_logs(Level::WARN);
    let outcome = support::run(&logging_obj, &lookup, SHORT_AUDIO, None, RECOGNIZED);
    assert_eq!(support::cost_of(&outcome), Decimal::ZERO);
    assert!(logs.text().contains("zero cost"));
}

#[test]
fn a_negative_duration_is_billed_without_asking_the_registry() {
    let logging_obj = support::logging_object();
    let lookup = support::RecordingLookup::unpriced();
    let outcome = support::run(
        &logging_obj,
        &lookup,
        FAST,
        None,
        br#"{"durationMilliseconds": -5000}"#,
    );
    assert_eq!(support::cost_of(&outcome), Decimal::ZERO);
    assert!(lookup.calls().is_empty());
}

#[test]
fn a_batch_route_never_asks_for_a_price() {
    let logging_obj = support::logging_object();
    let lookup = support::RecordingLookup::unpriced();
    let outcome = support::run(&logging_obj, &lookup, BATCH, None, RECOGNIZED);
    assert_eq!(support::cost_of(&outcome), Decimal::ZERO);
    assert!(lookup.calls().is_empty());
}
