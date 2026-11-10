#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::datadog_llm_privacy::latency::{latency_metrics, time_to_first_token_seconds};
use crate::datadog_llm_privacy::record_object;
use crate::datadog_llm_privacy::tests::support;
use crate::fields::payload::StandardLoggingPayload;
use crate::fields::payload::records::{DecimalValue, GuardrailInformation};

fn guardrail(duration: &str) -> GuardrailInformation {
    serde_json::from_str(&format!(r#"{{"duration": {duration}}}"#)).expect("the record parses")
}

fn metrics(payload: &StandardLoggingPayload) -> Json {
    let object =
        record_object("latency_metrics", &latency_metrics(payload)).expect("the metrics encode");
    Json::Object(object)
}

#[rstest]
#[case::completion_start(Some(100.0_f64), Some(100.5_f64), Some(102.0_f64), 0.5_f64)]
#[case::falls_back_to_end(Some(100.0_f64), None, Some(102.0_f64), 2.0_f64)]
#[case::no_start(None, Some(100.5_f64), Some(102.0_f64), 0.0_f64)]
#[case::no_times(None, None, None, 0.0_f64)]
#[case::completion_start_without_end(Some(100.0_f64), Some(100.25_f64), None, 0.25_f64)]
#[case::end_without_completion_start(Some(100.0_f64), None, None, 0.0_f64)]
fn time_to_first_token_seconds_reads_the_first_available_pair(
    #[case] start_time: Option<f64>,
    #[case] completion_start_time: Option<f64>,
    #[case] end_time: Option<f64>,
    #[case] expected: f64,
) {
    let seconds = time_to_first_token_seconds(start_time, completion_start_time, end_time);
    assert_eq!(seconds.to_bits(), expected.to_bits());
}

#[test]
fn latency_metrics_cover_token_time_overhead_and_guardrails() {
    let mut payload = support::payload();
    payload.start_time = Decimal::new(1_704_067_200, 0);
    payload.completion_start_time = Decimal::new(1_704_067_200_250, 3);
    payload.end_time = Decimal::new(1_704_067_202, 0);
    payload.hidden_params.litellm_overhead_time_ms = Some(DecimalValue(Decimal::new(125, 1)));
    payload.guardrail_information = Some(vec![guardrail("0.25"), guardrail("0.5")]);
    let object = metrics(&payload);
    assert_eq!(
        object,
        support::json(
            r#"{"time_to_first_token_ms": 250.0, "litellm_overhead_time_ms": 12.5, "guardrail_overhead_time_ms": 750.0}"#
        )
    );
}

#[test]
fn latency_metrics_omit_a_zero_token_time_and_a_negative_total() {
    let mut payload = support::payload();
    payload.completion_start_time = payload.start_time;
    payload.hidden_params.litellm_overhead_time_ms = None;
    payload.guardrail_information = Some(vec![guardrail("0"), guardrail("-0.5")]);
    let object = metrics(&payload);
    assert_eq!(object, support::json("{}"));
}

#[test]
fn latency_metrics_omit_a_zero_guardrail_total() {
    let mut payload = support::payload();
    payload.completion_start_time = payload.start_time;
    payload.guardrail_information = Some(vec![guardrail("0.5"), guardrail("-0.5")]);
    let object = metrics(&payload);
    assert_eq!(object, support::json("{}"));
}

#[test]
fn latency_metrics_keep_a_zero_overhead_the_payload_carries() {
    let mut payload = support::payload();
    payload.completion_start_time = payload.start_time;
    payload.hidden_params.litellm_overhead_time_ms = Some(DecimalValue(Decimal::ZERO));
    let object = metrics(&payload);
    assert_eq!(
        object,
        support::json(r#"{"litellm_overhead_time_ms": 0.0}"#)
    );
}
