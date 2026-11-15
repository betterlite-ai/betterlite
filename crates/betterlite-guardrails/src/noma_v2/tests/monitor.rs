use axum::http::StatusCode;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;

use crate::support::{
    BLOCKED_REPLY, captured_body, field, guardrail, inputs, mock_noma, records, request_count,
    request_data, scan, settings, text_field, value,
};

#[tokio::test]
async fn monitor_mode_allows_all() {
    let mock = mock_noma(StatusCode::OK, BLOCKED_REPLY).await;
    let mut config = settings(&mock.base);
    config.monitor_mode = true;
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
        Some("success")
    );
    assert_eq!(
        field(record, "guardrail_response"),
        Some(&value(BLOCKED_REPLY))
    );
    assert_eq!(
        captured_body(&mock).get("monitor_mode"),
        Some(&GuardrailValue::Boolean(true))
    );
    assert_eq!(request_count(&mock), 1);
}

#[tokio::test]
async fn monitor_mode_still_scans_and_records_the_response() {
    let reply = r#"{"action": "GUARDRAIL_INTERVENED", "texts": ["masked"]}"#;
    let mock = mock_noma(StatusCode::OK, reply).await;
    let mut config = settings(&mock.base);
    config.monitor_mode = true;
    let outcome = scan(
        &guardrail(config),
        inputs(),
        request_data(),
        GuardrailInputType::Request,
    )
    .await;
    let unchanged = outcome.result.unwrap();
    assert_eq!(unchanged.texts, inputs().texts);
    let record = &records(&outcome.request_data)[0];
    assert_eq!(
        text_field(record, "guardrail_status").as_deref(),
        Some("success")
    );
    assert_eq!(field(record, "guardrail_response"), Some(&value(reply)));
    assert_eq!(request_count(&mock), 1);
}

#[tokio::test]
async fn monitor_mode_ignores_a_malformed_action() {
    let mock = mock_noma(StatusCode::OK, r#"{"action": "MAYBE"}"#).await;
    let mut config = settings(&mock.base);
    config.monitor_mode = true;
    let outcome = scan(
        &guardrail(config),
        inputs(),
        request_data(),
        GuardrailInputType::Request,
    )
    .await;
    assert_eq!(outcome.result.unwrap(), inputs());
}
