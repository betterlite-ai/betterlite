use axum::http::StatusCode;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;

use crate::support::{
    NONE_REPLY, backwards_now, data, field, guardrail, guardrail_with, guardrail_with_clock,
    inputs, mock_noma, params, records, request_data, scan, settings, text_field, ticking_now,
    value,
};

#[tokio::test]
async fn the_observability_record_carries_provider_status_and_timing() {
    let mock = mock_noma(StatusCode::OK, NONE_REPLY).await;
    let outcome = scan(
        &guardrail_with_clock(settings(&mock.base), ticking_now),
        inputs(),
        request_data(),
        GuardrailInputType::Request,
    )
    .await;
    assert!(outcome.result.is_ok());
    let record = &records(&outcome.request_data)[0];
    assert_eq!(
        text_field(record, "guardrail_name").as_deref(),
        Some("noma-v2-test")
    );
    assert_eq!(
        text_field(record, "guardrail_provider").as_deref(),
        Some("noma_v2")
    );
    assert_eq!(
        text_field(record, "guardrail_status").as_deref(),
        Some("success")
    );
    assert_eq!(
        text_field(record, "guardrail_mode").as_deref(),
        Some("pre_call")
    );
    assert_eq!(
        field(record, "start_time"),
        Some(&GuardrailValue::Decimal(1_700_000_000.0))
    );
    assert_eq!(
        field(record, "end_time"),
        Some(&GuardrailValue::Decimal(1_700_000_001.0))
    );
}

#[tokio::test]
async fn the_observability_record_carries_the_scan_response_and_duration() {
    let mock = mock_noma(StatusCode::OK, NONE_REPLY).await;
    let outcome = scan(
        &guardrail(settings(&mock.base)),
        inputs(),
        request_data(),
        GuardrailInputType::Request,
    )
    .await;
    assert!(outcome.result.is_ok());
    let record = &records(&outcome.request_data)[0];
    assert_eq!(
        field(record, "guardrail_response"),
        Some(&value(NONE_REPLY))
    );
    assert_eq!(
        field(record, "duration"),
        Some(&GuardrailValue::Decimal(0.0))
    );
}

#[tokio::test]
async fn a_backwards_clock_fails_the_record() {
    let mock = mock_noma(StatusCode::OK, NONE_REPLY).await;
    let outcome = scan(
        &guardrail_with_clock(settings(&mock.base), backwards_now),
        inputs(),
        request_data(),
        GuardrailInputType::Request,
    )
    .await;
    let Err(GuardrailFailure::Other(message)) = outcome.result else {
        panic!("the backwards clock was not refused");
    };
    assert_eq!(message, "the system clock is set before the Unix epoch");
    assert!(records(&outcome.request_data).is_empty());
}

#[tokio::test]
async fn the_observability_record_goes_into_metadata_without_litellm_metadata() {
    let mock = mock_noma(StatusCode::OK, NONE_REPLY).await;
    let outcome = scan(
        &guardrail(settings(&mock.base)),
        inputs(),
        data(r#"{"model": "gpt-4o"}"#),
        GuardrailInputType::Request,
    )
    .await;
    assert!(!outcome.request_data.contains_key("litellm_metadata"));
    assert!(outcome.request_data.contains_key("metadata"));
    let record = &records(&outcome.request_data)[0];
    assert_eq!(
        text_field(record, "guardrail_provider").as_deref(),
        Some("noma_v2")
    );
}

#[tokio::test]
async fn the_logging_object_receives_the_record() {
    let mock = mock_noma(StatusCode::OK, NONE_REPLY).await;
    let outcome = scan(
        &guardrail(settings(&mock.base)),
        inputs(),
        request_data(),
        GuardrailInputType::Request,
    )
    .await;
    assert!(outcome.result.is_ok());
    let Some(logging_params) = outcome.log_target.litellm_params.as_ref() else {
        panic!("the logging object has no litellm_params");
    };
    let Some(GuardrailValue::Object(metadata)) = logging_params.get("metadata") else {
        panic!("the logging object has no metadata bucket");
    };
    let Some(GuardrailValue::Array(entries)) =
        metadata.get("standard_logging_guardrail_information")
    else {
        panic!("the logging object has no guardrail records");
    };
    assert_eq!(entries.len(), 1);
    assert_eq!(
        text_field(&entries[0], "guardrail_provider").as_deref(),
        Some("noma_v2")
    );
    assert_eq!(
        text_field(&entries[0], "guardrail_status").as_deref(),
        Some("success")
    );
}

#[tokio::test]
async fn the_observability_record_takes_its_mode_from_the_configured_hook() {
    let mock = mock_noma(StatusCode::OK, NONE_REPLY).await;
    let mut configured = params();
    configured.mode = Some(LitellmParamsMode::One(String::from("post_call")));
    let outcome = scan(
        &guardrail_with(&configured, settings(&mock.base)),
        inputs(),
        request_data(),
        GuardrailInputType::Response,
    )
    .await;
    let record = &records(&outcome.request_data)[0];
    assert_eq!(
        text_field(record, "guardrail_mode").as_deref(),
        Some("post_call")
    );
}
