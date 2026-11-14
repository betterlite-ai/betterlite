use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::StatusCode;
use rstest::rstest;
use tokio::net::TcpListener;

use super::{
    FOUR, SERVICE_ONE, THREE_USERS, apply, data, default_input, fail_open_input, guardrail,
    guardrail_with_clock, inputs, json, mock, only_call, record_response, record_status,
};
use crate::guardrail_base::failure::GuardrailFailure;
use crate::headroom_failure::NOT_FOUND_HINT;
use crate::headroom_settings::HeadroomInput;

use super::super::apply::UNAVAILABLE_MESSAGE;
use super::super::compress::{
    BODY_PREVIEW_CHARS, COUNT_ERROR, EMPTY_MESSAGES_ERROR, MISSING_MESSAGES_ERROR, NON_JSON_ERROR,
    SERVICE_ERROR, SHAPE_ERROR, UNREACHABLE_ERROR,
};

async fn recorded_response(payload: &str) -> serde_json::Value {
    let server = mock(StatusCode::OK, payload.to_owned()).await;
    let guardrail = guardrail(&server.base);
    let mut request = data("{}");
    apply(&guardrail, inputs(FOUR), &mut request).await.unwrap();
    record_response(&request)
}

fn detail_of(error: GuardrailFailure) -> (u16, BTreeMap<String, GuardrailValue>) {
    let GuardrailFailure::HttpObject { status, detail } = error else {
        panic!("expected an HTTP failure, got {error}");
    };
    (status, detail.into_iter().collect())
}

#[tokio::test]
async fn the_compression_request_posts_flattened_messages() {
    let server = mock(StatusCode::OK, SERVICE_ONE.to_owned()).await;
    let guardrail = guardrail(&server.base);
    let mut request = data("{}");
    apply(&guardrail, inputs(FOUR), &mut request).await.unwrap();
    let call = only_call(&server);
    assert_eq!(call.method, "POST");
    assert_eq!(call.path, "/v1/compress");
    assert_eq!(call.authorization.as_deref(), Some("Bearer sk-test"));
    assert_eq!(call.content_type.as_deref(), Some("application/json"));
    assert_eq!(
        json(&call.body),
        json(r#"{"messages":[{"role":"user","content":"long text"}]}"#)
    );
}

#[tokio::test]
async fn the_compression_request_sends_the_configured_model() {
    let server = mock(StatusCode::OK, SERVICE_ONE.to_owned()).await;
    let input = HeadroomInput {
        model: Some("compressor".to_owned()),
        ..default_input(&server.base)
    };
    let guardrail = guardrail_with_clock(input);
    let mut request = data(r#"{"model":"gpt-4o"}"#);
    apply(&guardrail, inputs(FOUR), &mut request).await.unwrap();
    let body = json(&only_call(&server).body);
    assert_eq!(
        body.get("model"),
        Some(&serde_json::Value::String("compressor".to_owned()))
    );
}

#[tokio::test]
async fn an_empty_configured_model_falls_back_to_the_request_model() {
    let server = mock(StatusCode::OK, SERVICE_ONE.to_owned()).await;
    let input = HeadroomInput {
        model: Some(String::new()),
        ..default_input(&server.base)
    };
    let guardrail = guardrail_with_clock(input);
    let mut request = data(r#"{"model":"gpt-4o"}"#);
    apply(&guardrail, inputs(FOUR), &mut request).await.unwrap();
    let body = json(&only_call(&server).body);
    assert_eq!(
        body.get("model"),
        Some(&serde_json::Value::String("gpt-4o".to_owned()))
    );
}

#[tokio::test]
async fn an_empty_request_model_is_not_sent() {
    let server = mock(StatusCode::OK, SERVICE_ONE.to_owned()).await;
    let guardrail = guardrail(&server.base);
    let mut request = data(r#"{"model":""}"#);
    apply(&guardrail, inputs(FOUR), &mut request).await.unwrap();
    let body = json(&only_call(&server).body);
    assert!(body.get("model").is_none());
}

#[tokio::test]
async fn the_compression_request_falls_back_to_the_request_model() {
    let server = mock(StatusCode::OK, SERVICE_ONE.to_owned()).await;
    let guardrail = guardrail(&server.base);
    let mut request = data(r#"{"model":"gpt-4o"}"#);
    apply(&guardrail, inputs(FOUR), &mut request).await.unwrap();
    let body = json(&only_call(&server).body);
    assert_eq!(
        body.get("model"),
        Some(&serde_json::Value::String("gpt-4o".to_owned()))
    );
}

#[tokio::test]
async fn the_compression_request_omits_an_unknown_model() {
    let server = mock(StatusCode::OK, SERVICE_ONE.to_owned()).await;
    let guardrail = guardrail(&server.base);
    let mut request = data("{}");
    apply(&guardrail, inputs(FOUR), &mut request).await.unwrap();
    let body = json(&only_call(&server).body);
    assert!(body.get("model").is_none());
}

#[tokio::test]
async fn the_compression_request_omits_authorization_without_a_key() {
    let server = mock(StatusCode::OK, SERVICE_ONE.to_owned()).await;
    let input = HeadroomInput {
        api_key: None,
        ..default_input(&server.base)
    };
    let guardrail = guardrail_with_clock(input);
    let mut request = data("{}");
    apply(&guardrail, inputs(FOUR), &mut request).await.unwrap();
    assert!(only_call(&server).authorization.is_none());
}

#[tokio::test]
async fn statistics_copy_the_five_keys_and_drop_the_rest() {
    let response = recorded_response(
        r#"{"messages":[{"role":"user","content":"short"}],"tokens_before":100,"tokens_after":25,"tokens_saved":50,"compression_ratio":4.0,"transforms_applied":["dedupe"],"unused":1}"#,
    )
    .await;
    assert_eq!(
        response,
        json(
            r#"{"tokens_before":100,"tokens_after":25,"tokens_saved":50,"compression_ratio":4.0,"transforms_applied":["dedupe"]}"#
        )
    );
}

#[rstest]
#[case::integers(
    r#"{"messages":[{"role":"user","content":"short"}],"tokens_before":100,"tokens_after":25}"#,
    75
)]
#[case::present_wins(
    r#"{"messages":[{"role":"user","content":"short"}],"tokens_before":100,"tokens_after":25,"tokens_saved":1}"#,
    1
)]
#[case::negative(
    r#"{"messages":[{"role":"user","content":"short"}],"tokens_before":10,"tokens_after":25}"#,
    -15
)]
#[tokio::test]
async fn tokens_saved_is_derived_from_both_counts(#[case] payload: &str, #[case] expected: i64) {
    let response = recorded_response(payload).await;
    assert_eq!(
        response.get("tokens_saved"),
        Some(&serde_json::Value::Number(serde_json::Number::from(
            expected
        )))
    );
}

#[tokio::test]
async fn tokens_saved_is_derived_from_float_counts() {
    let response = recorded_response(
        r#"{"messages":[{"role":"user","content":"short"}],"tokens_before":100.5,"tokens_after":25.25}"#,
    )
    .await;
    assert_eq!(
        response.get("tokens_saved"),
        Some(&serde_json::json!(75.25))
    );
}

#[rstest]
#[case::booleans(
    r#"{"messages":[{"role":"user","content":"short"}],"tokens_before":true,"tokens_after":25}"#
)]
#[case::strings(
    r#"{"messages":[{"role":"user","content":"short"}],"tokens_before":"100","tokens_after":25}"#
)]
#[case::missing_after(r#"{"messages":[{"role":"user","content":"short"}],"tokens_before":100}"#)]
#[tokio::test]
async fn tokens_saved_is_omitted_without_two_numbers(#[case] payload: &str) {
    let response = recorded_response(payload).await;
    assert!(response.get("tokens_saved").is_none());
}

#[rstest]
#[case::non_json("not json at all", NON_JSON_ERROR)]
#[case::non_object("[1,2]", SHAPE_ERROR)]
#[case::missing_messages(r#"{"other":1}"#, MISSING_MESSAGES_ERROR)]
#[case::messages_not_a_list(r#"{"messages":"nope"}"#, MISSING_MESSAGES_ERROR)]
#[case::empty_messages(r#"{"messages":[]}"#, EMPTY_MESSAGES_ERROR)]
#[case::no_object_rows(r#"{"messages":[1,2]}"#, EMPTY_MESSAGES_ERROR)]
#[tokio::test]
async fn malformed_responses_fall_open_with_the_reason(
    #[case] payload: &str,
    #[case] reason: &str,
) {
    let server = mock(StatusCode::OK, payload.to_owned()).await;
    let guardrail = guardrail_with_clock(fail_open_input(&server.base));
    let original = inputs(FOUR);
    let mut request = data("{}");
    let returned = apply(&guardrail, original.clone(), &mut request)
        .await
        .unwrap();
    assert_eq!(returned, original);
    assert_eq!(
        record_response(&request),
        serde_json::json!({"error": reason})
    );
    assert_eq!(
        record_status(&request),
        serde_json::Value::String("guardrail_failed_to_respond".to_owned())
    );
}

#[tokio::test]
async fn a_changed_message_count_fails_closed_with_both_counts() {
    let server = mock(
        StatusCode::OK,
        r#"{"messages":[{"role":"user","content":"a"}]}"#.to_owned(),
    )
    .await;
    let guardrail = guardrail(&server.base);
    let mut request = data("{}");
    let error = apply(&guardrail, inputs(THREE_USERS), &mut request)
        .await
        .unwrap_err();
    let (status, detail) = detail_of(error);
    assert_eq!(status, 502);
    assert_eq!(
        detail.get("error"),
        Some(&GuardrailValue::Text(COUNT_ERROR.to_owned()))
    );
    assert_eq!(detail.get("sent"), Some(&GuardrailValue::SignedInteger(2)));
    assert_eq!(
        detail.get("returned"),
        Some(&GuardrailValue::SignedInteger(1))
    );
    assert!(records_are_absent(&request));
}

#[tokio::test]
async fn an_http_error_status_fails_closed_with_the_body() {
    let server = mock(StatusCode::INTERNAL_SERVER_ERROR, "boom".to_owned()).await;
    let guardrail = guardrail(&server.base);
    let mut request = data("{}");
    let error = apply(&guardrail, inputs(FOUR), &mut request)
        .await
        .unwrap_err();
    let (status, detail) = detail_of(error);
    assert_eq!(status, 502);
    assert_eq!(
        detail.get("error"),
        Some(&GuardrailValue::Text(SERVICE_ERROR.to_owned()))
    );
    assert_eq!(
        detail.get("status_code"),
        Some(&GuardrailValue::SignedInteger(500))
    );
    assert_eq!(
        detail.get("body"),
        Some(&GuardrailValue::Text("boom".to_owned()))
    );
    assert!(detail.get("hint").is_none());
    assert!(records_are_absent(&request));
}

#[tokio::test]
async fn a_404_response_adds_the_endpoint_hint() {
    let server = mock(StatusCode::NOT_FOUND, "gone".to_owned()).await;
    let guardrail = guardrail(&server.base);
    let mut request = data("{}");
    let error = apply(&guardrail, inputs(FOUR), &mut request)
        .await
        .unwrap_err();
    let (_status, detail) = detail_of(error);
    assert_eq!(
        detail.get("hint"),
        Some(&GuardrailValue::Text(NOT_FOUND_HINT.to_owned()))
    );
}

#[tokio::test]
async fn a_non_json_body_is_previewed_in_the_failure_detail() {
    let long = "x".repeat(BODY_PREVIEW_CHARS * 2);
    let server = mock(StatusCode::OK, long).await;
    let guardrail = guardrail(&server.base);
    let mut request = data("{}");
    let error = apply(&guardrail, inputs(FOUR), &mut request)
        .await
        .unwrap_err();
    let (_status, detail) = detail_of(error);
    let Some(GuardrailValue::Text(body)) = detail.get("body") else {
        panic!("a body preview");
    };
    assert_eq!(body.chars().count(), BODY_PREVIEW_CHARS);
}

#[rstest]
#[case::nan("NaN")]
#[case::infinity("Infinity")]
#[case::negative_infinity("-Infinity")]
#[tokio::test]
async fn non_finite_json_numbers_are_rejected_as_non_json(#[case] literal: &str) {
    let payload = format!(
        r#"{{"messages":[{{"role":"user","content":"short"}}],"tokens_before":{literal}}}"#
    );
    let server = mock(StatusCode::OK, payload).await;
    let guardrail = guardrail(&server.base);
    let mut request = data("{}");
    let error = apply(&guardrail, inputs(FOUR), &mut request)
        .await
        .unwrap_err();
    let (_status, detail) = detail_of(error);
    assert_eq!(
        detail.get("error"),
        Some(&GuardrailValue::Text(NON_JSON_ERROR.to_owned()))
    );
}

#[tokio::test]
async fn a_missing_messages_field_keeps_the_whole_body() {
    let long = format!(r#"{{"other":"{}"}}"#, "x".repeat(BODY_PREVIEW_CHARS * 2));
    let server = mock(StatusCode::OK, long).await;
    let guardrail = guardrail(&server.base);
    let mut request = data("{}");
    let error = apply(&guardrail, inputs(FOUR), &mut request)
        .await
        .unwrap_err();
    let (_status, detail) = detail_of(error);
    let Some(GuardrailValue::Text(body)) = detail.get("body") else {
        panic!("a body");
    };
    assert!(body.chars().count() > BODY_PREVIEW_CHARS);
}

#[tokio::test]
async fn an_unreachable_service_fails_closed() {
    let guardrail = guardrail("http://127.0.0.1:1");
    let mut request = data("{}");
    let error = apply(&guardrail, inputs(FOUR), &mut request)
        .await
        .unwrap_err();
    let (status, detail) = detail_of(error);
    assert_eq!(status, 502);
    assert_eq!(
        detail.get("error"),
        Some(&GuardrailValue::Text(UNREACHABLE_ERROR.to_owned()))
    );
}

#[tokio::test]
async fn an_unreachable_service_falls_open() {
    let guardrail = guardrail_with_clock(fail_open_input("http://127.0.0.1:1"));
    let original = inputs(FOUR);
    let mut request = data("{}");
    let returned = apply(&guardrail, original.clone(), &mut request)
        .await
        .unwrap();
    assert_eq!(returned, original);
    assert_eq!(
        record_response(&request),
        serde_json::json!({"error": UNAVAILABLE_MESSAGE})
    );
}

#[tokio::test]
async fn a_timeout_fails_open() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let input = HeadroomInput {
        timeout: Some(0.1),
        ..fail_open_input(&base)
    };
    let guardrail = guardrail_with_clock(input);
    let original = inputs(FOUR);
    let mut request = data("{}");
    let returned = apply(&guardrail, original.clone(), &mut request)
        .await
        .unwrap();
    assert_eq!(returned, original);
    assert_eq!(
        record_response(&request),
        serde_json::json!({"error": UNAVAILABLE_MESSAGE})
    );
}

#[tokio::test]
async fn a_timeout_fails_closed() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let input = HeadroomInput {
        timeout: Some(0.1),
        ..default_input(&base)
    };
    let guardrail = guardrail_with_clock(input);
    let mut request = data("{}");
    let error = apply(&guardrail, inputs(FOUR), &mut request)
        .await
        .unwrap_err();
    let (status, detail) = detail_of(error);
    assert_eq!(status, 502);
    assert_eq!(
        detail.get("error"),
        Some(&GuardrailValue::Text(UNREACHABLE_ERROR.to_owned()))
    );
}

fn records_are_absent(request: &BTreeMap<String, GuardrailValue>) -> bool {
    super::records(request).is_empty()
}

#[test]
fn updating_params_re_resolves_and_re_clamps_the_timeout() {
    let mut guardrail = guardrail("http://127.0.0.1:1");
    assert_eq!(guardrail.settings.timeout.total, Duration::from_secs(60));
    assert_eq!(guardrail.settings.timeout.connect, Duration::from_secs(5));
    guardrail.update_in_memory_params(Some(2.0));
    assert_eq!(guardrail.settings.timeout.total, Duration::from_secs(2));
    assert_eq!(guardrail.settings.timeout.connect, Duration::from_secs(2));
    guardrail.update_in_memory_params(Some(10.0));
    assert_eq!(guardrail.settings.timeout.total, Duration::from_secs(10));
    assert_eq!(guardrail.settings.timeout.connect, Duration::from_secs(5));
    guardrail.update_in_memory_params(Some(0.0));
    assert_eq!(guardrail.settings.timeout.total, Duration::from_secs(60));
    assert_eq!(guardrail.settings.timeout.connect, Duration::from_secs(5));
    guardrail.update_in_memory_params(None);
    assert_eq!(guardrail.settings.timeout.total, Duration::from_secs(60));
}
