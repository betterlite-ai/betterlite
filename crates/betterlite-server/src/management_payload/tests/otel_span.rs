#![cfg(test)]

use betterlite_core::api::request_body::RequestBodyError;
use betterlite_logging::otel::legacy_policy::span_time::SpanTime;
use betterlite_logging::otel::v2_callback::live_span::LiveSpan;
use betterlite_models::decisions::Json;
use opentelemetry::trace::{Tracer as _, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use rstest::rstest;
use serde_json::json;

use crate::management_payload::otel_span::ManagementSpanError;
use crate::management_payload::tests::support::{
    FakeTransportError, SpanHarness, SpanHook, emit_without_transport, end_time, exception,
    json_request, json_value, span_call, start_time,
};

fn parent_span() -> LiveSpan {
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(InMemorySpanExporter::default())
        .build();
    let tracer = provider.tracer("management-payload");
    LiveSpan::from(tracer.start("parent"))
}

#[tokio::test]
async fn legacy_span_without_request() {
    let harness = SpanHarness::default();
    let result = json_value(json!({"key_alias": "ops", "env_vars": []}));
    harness
        .emit(span_call("delete_user", Some(&result), None))
        .await
        .expect("the span is sent");
    let sent = harness.transport.spans.entries();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].hook, SpanHook::Success);
    assert_eq!(sent[0].route, "delete_user");
    assert_eq!(sent[0].request_data, json_value(json!({})));
    assert_eq!(
        sent[0].response,
        Some(json_value(json!({"key_alias": "ops", "env_vars": []})))
    );
}

#[tokio::test]
async fn request_route_and_body() {
    let harness = SpanHarness::default();
    let result = json_value(json!({"key": "sk-generated", "user_id": "u1"}));
    let mut call = span_call("generate_key_fn", Some(&result), None);
    call.http_request = Some(json_request(
        "/key/generate",
        r#"{"user_id": "u1", "api_key": "sk-request-secret"}"#,
    ));
    harness.emit(call).await.expect("the span is sent");
    let sent = harness.transport.spans.entries();
    assert_eq!(sent[0].route, "/key/generate");
    // the request body keeps every field it carried; only the response is stripped
    assert_eq!(
        sent[0].request_data,
        json_value(json!({"user_id": "u1", "api_key": "sk-request-secret"}))
    );
    assert_eq!(sent[0].response, Some(json_value(json!({"user_id": "u1"}))));
}

#[tokio::test]
async fn otel_v2_skips_all_payload_work() {
    let harness = SpanHarness::default();
    let result = json_value(json!({"key": "sk-generated"}));
    // the body would fail to parse, so a completed call proves the parser never ran
    let mut skipped = span_call("generate_key_fn", Some(&result), None);
    skipped.http_request = Some(json_request("/key/generate", "not json"));
    harness
        .emit_v2(skipped)
        .await
        .expect("v2 builds no payload");
    let mut absent = span_call("generate_key_fn", Some(&result), None);
    absent.http_request = Some(json_request("/key/generate", "not json"));
    emit_without_transport(absent)
        .await
        .expect("a missing logger skips the payload");
    assert!(harness.transport.spans.entries().is_empty());
}

#[tokio::test]
async fn top_level_credential_fields_only() {
    let harness = SpanHarness::default();
    let result = json_value(json!({
        "key": "sk-top",
        "token": "top-token",
        "api_key": "top-api-key",
        "secret": "top-secret",
        "password": "top-password",
        "access_token": "top-access",
        "refresh_token": "top-refresh",
        "private_key": "top-private",
        "service_account_key": "top-service",
        "key_alias": "ops",
        "nested": {"api_key": "sk-nested", "secret": "nested-secret"},
        "env_vars": [{"name": "A", "value": "env-secret", "scope": "global"}]
    }));
    harness
        .emit(span_call("update_key_fn", Some(&result), None))
        .await
        .expect("the span is sent");
    let sent = harness.transport.spans.entries();
    assert_eq!(
        sent[0].response,
        Some(json_value(json!({
            "key_alias": "ops",
            "nested": {"api_key": "sk-nested", "secret": "nested-secret"},
            "env_vars": [
                {"name": "A", "scope": "global", "description": null, "value": ""}
            ]
        })))
    );
}

#[rstest]
#[case::text_result(json_value(json!("not a mapping")))]
#[case::list_result(json_value(json!([["key", 1]])))]
#[tokio::test]
async fn response_conversion_failure_silent(#[case] value: Json) {
    let harness = SpanHarness::default();
    harness
        .emit(span_call("generate_key_fn", Some(&value), None))
        .await
        .expect("a non-mapping result is silent");
    let sent = harness.transport.spans.entries();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].hook, SpanHook::Success);
    assert_eq!(sent[0].response, None);
}

#[tokio::test]
async fn success_null_result_and_failure_payload() {
    let harness = SpanHarness::default();
    harness
        .emit(span_call("delete_key_fn", None, None))
        .await
        .expect("an absent result is no response");
    let nulled = json_value(json!(null));
    harness
        .emit(span_call("delete_key_fn", Some(&nulled), None))
        .await
        .expect("a null result is no response");
    let facts = exception();
    let secret = json_value(json!({"key": "sk-failed", "env_vars": [{"name": "A", "value": "x"}]}));
    harness
        .emit(span_call("delete_key_fn", Some(&secret), Some(&facts)))
        .await
        .expect("the failure hook is sent");
    let sent = harness.transport.spans.entries();
    assert_eq!(sent.len(), 3);
    assert_eq!(sent[0].hook, SpanHook::Success);
    assert_eq!(sent[0].response, None);
    assert_eq!(sent[1].hook, SpanHook::Success);
    assert_eq!(sent[1].response, None);
    // a failure carries no response, whatever result the call also passed
    assert_eq!(sent[2].hook, SpanHook::Failure);
    assert_eq!(sent[2].response, None);
    let carried = sent[2]
        .exception
        .as_ref()
        .expect("the exception is preserved");
    assert_eq!(carried.class_name, "ProxyException");
    assert_eq!(carried.message.as_deref(), Some("boom"));
}

#[tokio::test]
async fn timestamps_and_optional_parent_preserved() {
    let harness = SpanHarness::default();
    let parent = parent_span();
    let result = json_value(json!({"key_alias": "ops"}));
    let mut with_parent = span_call("new_team", Some(&result), None);
    with_parent.parent_otel_span = Some(&parent);
    harness
        .emit(with_parent)
        .await
        .expect("the parented span is sent");
    harness
        .emit(span_call("new_team", Some(&result), None))
        .await
        .expect("the orphan span is sent");
    let sent = harness.transport.spans.entries();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].start_time, SpanTime::DateTime(start_time()));
    assert_eq!(sent[0].end_time, SpanTime::DateTime(end_time()));
    assert_eq!(sent[1].start_time, SpanTime::DateTime(start_time()));
    assert_eq!(sent[1].end_time, SpanTime::DateTime(end_time()));
    assert_eq!(sent[0].parent, Some(parent.context.clone()));
    assert_eq!(sent[1].parent, None);
}

#[tokio::test]
async fn body_parser_failure_propagates() {
    let harness = SpanHarness::default();
    let mut call = span_call("generate_key_fn", None, None);
    call.http_request = Some(json_request("/key/generate", "not json"));
    let outcome = harness.emit(call).await;
    assert!(matches!(
        outcome,
        Err(ManagementSpanError::Body(RequestBodyError::InvalidJson))
    ));
    assert!(harness.transport.spans.entries().is_empty());
}

#[tokio::test]
async fn span_sender_failure_propagates() {
    let harness = SpanHarness::failing();
    let result = json_value(json!({"key_alias": "ops"}));
    let success = harness
        .emit(span_call("new_team", Some(&result), None))
        .await;
    assert!(matches!(
        success,
        Err(ManagementSpanError::Span(FakeTransportError))
    ));
    let facts = exception();
    let failure = harness
        .emit(span_call("new_team", None, Some(&facts)))
        .await;
    assert!(matches!(
        failure,
        Err(ManagementSpanError::Span(FakeTransportError))
    ));
    assert!(harness.transport.spans.entries().is_empty());
}

#[tokio::test]
async fn unchanged_caller_response() {
    let harness = SpanHarness::default();
    let result = json_value(json!({
        "key": "sk-live",
        "env_vars": [{"name": "A", "value": "env-secret", "scope": "global"}],
        "items": [{"env_vars": [{"name": "B", "value": "item-secret"}]}]
    }));
    let before = result.clone();
    harness
        .emit(span_call("update_key_fn", Some(&result), None))
        .await
        .expect("the span is sent");
    // the response the route caller holds is untouched
    assert_eq!(result, before);
    let sent = harness.transport.spans.entries();
    assert_eq!(
        sent[0].response,
        Some(json_value(json!({
            "env_vars": [
                {"name": "A", "scope": "global", "description": null, "value": ""}
            ],
            "items": [
                {"env_vars": [
                    {"name": "B", "scope": null, "description": null, "value": ""}
                ]}
            ]
        })))
    );
}
