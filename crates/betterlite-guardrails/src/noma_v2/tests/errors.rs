use std::time::Duration;

use axum::http::StatusCode;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::noma_scan_policy::BLOCKED_MESSAGE;

use crate::support::{
    BLOCKED_REPLY, NONE_REPLY, closed_base, data, field, guardrail, inputs, mock_noma,
    mock_noma_after, records, request_count, request_data, scan, settings, text_field, value,
};

// Hardcoded independently of the guardrail's cap, so a mutation of that constant cannot move the boundary too.
const SPEC_MAX_RESPONSE_BYTES: usize = 0x0010_0000;

#[tokio::test]
async fn error_handling() {
    let mock = mock_noma(StatusCode::INTERNAL_SERVER_ERROR, "boom").await;
    let outcome = scan(
        &guardrail(settings(&mock.base)),
        inputs(),
        request_data(),
        GuardrailInputType::Request,
    )
    .await;
    let Err(GuardrailFailure::Other(message)) = outcome.result else {
        panic!("the scan failure was not raised");
    };
    assert_eq!(
        message,
        "the Noma API returned status 500 Internal Server Error"
    );
    let record = &records(&outcome.request_data)[0];
    assert_eq!(
        text_field(record, "guardrail_status").as_deref(),
        Some("guardrail_failed_to_respond")
    );
    assert_eq!(
        field(record, "guardrail_response"),
        Some(&GuardrailValue::Text(message))
    );
}

#[tokio::test]
async fn error_handling_blocked() {
    let mock = mock_noma(StatusCode::OK, BLOCKED_REPLY).await;
    let mut config = settings(&mock.base);
    config.block_failures = false;
    let outcome = scan(
        &guardrail(config),
        inputs(),
        request_data(),
        GuardrailInputType::Request,
    )
    .await;
    let Err(GuardrailFailure::HttpObject { status, detail }) = outcome.result else {
        panic!("a BLOCKED action was not raised with block_failures off");
    };
    assert_eq!(status, 400);
    assert_eq!(
        detail.get("error"),
        Some(&GuardrailValue::Text(String::from(BLOCKED_MESSAGE)))
    );
    let record = &records(&outcome.request_data)[0];
    assert_eq!(
        text_field(record, "guardrail_status").as_deref(),
        Some("guardrail_intervened")
    );
    assert_eq!(
        field(record, "guardrail_response"),
        Some(&value(BLOCKED_REPLY))
    );
}

#[tokio::test]
async fn block_failures_true_raises_the_scan_error() {
    let mock = mock_noma(StatusCode::OK, "not json").await;
    let outcome = scan(
        &guardrail(settings(&mock.base)),
        inputs(),
        request_data(),
        GuardrailInputType::Request,
    )
    .await;
    let Err(GuardrailFailure::Other(message)) = outcome.result else {
        panic!("the unreadable response was not raised");
    };
    assert_eq!(message, "the Noma API response is not valid JSON");
}

#[tokio::test]
async fn block_failures_false_returns_the_inputs() {
    let mock = mock_noma(StatusCode::OK, "not json").await;
    let mut config = settings(&mock.base);
    config.block_failures = false;
    let outcome = scan(
        &guardrail(config),
        inputs(),
        request_data(),
        GuardrailInputType::Request,
    )
    .await;
    assert_eq!(outcome.result.unwrap(), inputs());
    let record = &records(&outcome.request_data)[0];
    assert_eq!(
        text_field(record, "guardrail_status").as_deref(),
        Some("guardrail_failed_to_respond")
    );
    assert_eq!(
        field(record, "guardrail_response"),
        Some(&GuardrailValue::Text(String::from(
            "the Noma API response is not valid JSON"
        )))
    );
}

#[tokio::test]
async fn a_timed_out_scan_fails_with_status_408() {
    let mock = mock_noma_after(Duration::from_millis(200), StatusCode::OK, NONE_REPLY).await;
    let mut config = settings(&mock.base);
    config.timeout = Some(Duration::from_millis(10));
    config.request_timeout = Duration::from_millis(10);
    let outcome = scan(
        &guardrail(config),
        inputs(),
        request_data(),
        GuardrailInputType::Request,
    )
    .await;
    let Err(GuardrailFailure::Http { status, detail }) = outcome.result else {
        panic!("the timed out scan was not raised with a status");
    };
    assert_eq!(status, 408);
    assert_eq!(detail, "the Noma v2 request timed out");
    let record = &records(&outcome.request_data)[0];
    assert_eq!(
        text_field(record, "guardrail_status").as_deref(),
        Some("guardrail_failed_to_respond")
    );
}

#[tokio::test]
async fn a_response_above_the_cap_fails_the_scan() {
    let reply = " ".repeat(SPEC_MAX_RESPONSE_BYTES + 1);
    let mock = mock_noma(StatusCode::OK, &reply).await;
    let outcome = scan(
        &guardrail(settings(&mock.base)),
        inputs(),
        request_data(),
        GuardrailInputType::Request,
    )
    .await;
    let Err(GuardrailFailure::Other(message)) = outcome.result else {
        panic!("the oversized response was not refused");
    };
    assert_eq!(
        message,
        format!("the Noma v2 response is larger than {SPEC_MAX_RESPONSE_BYTES} bytes")
    );
    let record = &records(&outcome.request_data)[0];
    assert_eq!(
        text_field(record, "guardrail_status").as_deref(),
        Some("guardrail_failed_to_respond")
    );
}

#[tokio::test]
async fn a_transport_error_fails_the_scan() {
    let outcome = scan(
        &guardrail(settings(&closed_base())),
        inputs(),
        request_data(),
        GuardrailInputType::Request,
    )
    .await;
    let Err(GuardrailFailure::Other(message)) = outcome.result else {
        panic!("the unreachable endpoint was not raised");
    };
    assert_eq!(message, "the Noma API could not be reached");
}

#[tokio::test]
async fn an_endpoint_that_is_not_a_url_fails_the_scan() {
    let outcome = scan(
        &guardrail(settings("noma-security")),
        inputs(),
        request_data(),
        GuardrailInputType::Request,
    )
    .await;
    let Err(GuardrailFailure::Other(message)) = outcome.result else {
        panic!("the malformed endpoint was not raised");
    };
    assert_eq!(message, "the Noma endpoint `noma-security` is not a URL");
}

#[tokio::test]
async fn a_broken_dynamic_param_fails_before_the_scan() {
    let mock = mock_noma(StatusCode::OK, NONE_REPLY).await;
    let sent_data = data(r#"{"guardrails": [{"noma-v2-test": {"extra_body": 5}}]}"#);
    let outcome = scan(
        &guardrail(settings(&mock.base)),
        inputs(),
        sent_data,
        GuardrailInputType::Request,
    )
    .await;
    let Err(GuardrailFailure::Http { status, detail }) = outcome.result else {
        panic!("the broken dynamic params were not refused");
    };
    assert_eq!(status, 400);
    assert_eq!(
        detail,
        "The request options for guardrail noma-v2-test must be an object"
    );
    assert_eq!(request_count(&mock), 0);
    assert!(records(&outcome.request_data).is_empty());
}
