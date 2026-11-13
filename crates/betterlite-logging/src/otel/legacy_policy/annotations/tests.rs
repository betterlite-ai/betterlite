#![cfg(test)]

use std::collections::BTreeMap;

use opentelemetry::Value;
use opentelemetry::trace::{Span as _, Tracer as _, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, Span};
use rstest::rstest;

use super::{
    REQUESTED_TIER_ATTRIBUTE, SERVED_TIER_ATTRIBUTE, set_service_tier_attributes,
    set_team_attributes, write_identity_attributes, write_team_attributes,
};
use crate::otel::model::utils::coercion::OtelModelValue;

fn payload_map(text: &str) -> BTreeMap<String, OtelModelValue> {
    serde_json::from_str(text).expect("payload")
}

fn record_attributes<F>(write: F) -> Vec<(String, Value)>
where
    F: FnOnce(&mut Span),
{
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let mut span = provider.tracer("legacy-policy-tests").start("test");
    write(&mut span);
    span.end();
    let spans = exporter.get_finished_spans().expect("spans");
    spans
        .first()
        .expect("one span")
        .attributes
        .iter()
        .map(|pair| (pair.key.to_string(), pair.value.clone()))
        .collect()
}

fn keys(attributes: &[(String, Value)]) -> Vec<&str> {
    attributes.iter().map(|(key, _)| key.as_str()).collect()
}

#[test]
fn team_attributes_skip_empty_and_missing_values() {
    let absent = record_attributes(|span| {
        set_team_attributes(span, Some(""), None);
    });
    assert!(absent.is_empty());
    let present = record_attributes(|span| {
        set_team_attributes(span, Some("team-id"), Some(""));
    });
    assert_eq!(
        present,
        vec![(
            "metadata.user_api_key_team_id".to_owned(),
            Value::from("team-id")
        )]
    );
}

#[test]
fn team_attributes_come_from_payload_metadata() {
    let payload = payload_map(
        r#"{"metadata": {"user_api_key_team_id": "team-id", "user_api_key_team_alias": "team-alias"}}"#,
    );
    let attributes = record_attributes(|span| {
        write_team_attributes(span, &payload);
    });
    assert_eq!(
        attributes,
        vec![
            (
                "metadata.user_api_key_team_id".to_owned(),
                Value::from("team-id")
            ),
            (
                "metadata.user_api_key_team_alias".to_owned(),
                Value::from("team-alias")
            ),
        ]
    );
}

#[test]
fn inference_identity_writes_the_groups_in_litellm_order() {
    let payload = payload_map(
        r#"{
            "metadata": {
                "user_api_key_request_route": "/v1/chat/completions",
                "requester_metadata": {"user_id": "u1"}
            },
            "model_group": "public-model",
            "model": "fallback/model",
            "hidden_params": {"litellm_model_name": "dispatched-model"}
        }"#,
    );
    let litellm_params = payload_map(
        r#"{"metadata": {"user_api_key_team_metadata": {"region": "west", "secret": "hidden"}}}"#,
    );
    let attributes = record_attributes(|span| {
        write_identity_attributes(
            span,
            &payload,
            &litellm_params,
            &["region".to_owned()],
            &["requester_metadata.user_id".to_owned()],
        );
    });
    assert_eq!(
        keys(&attributes),
        vec![
            "http.route",
            "litellm.team.metadata",
            "litellm.metadata.user_id",
            "litellm.model_group",
            "litellm.provider.model",
        ]
    );
    assert_eq!(
        attributes.get(1).map(|(_, value)| value),
        Some(&Value::from(r#"{"region":"west"}"#))
    );
}

#[test]
fn identity_team_metadata_falls_back_to_litellm_metadata() {
    let payload = payload_map("{}");
    let litellm_params = payload_map(
        r#"{"metadata": {}, "litellm_metadata": {"user_api_key_team_metadata": {"region": "west"}}}"#,
    );
    let attributes = record_attributes(|span| {
        write_identity_attributes(span, &payload, &litellm_params, &["region".to_owned()], &[]);
    });
    assert_eq!(keys(&attributes), vec!["litellm.team.metadata"]);
}

#[test]
fn identity_team_metadata_does_not_fall_back_from_a_non_empty_metadata_map() {
    let payload = payload_map("{}");
    let litellm_params = payload_map(
        r#"{"metadata": {"other": "value"}, "litellm_metadata": {"user_api_key_team_metadata": {"region": "west"}}}"#,
    );
    let attributes = record_attributes(|span| {
        write_identity_attributes(span, &payload, &litellm_params, &["region".to_owned()], &[]);
    });
    assert!(attributes.is_empty());
}

#[test]
fn identity_team_metadata_is_dropped_without_an_allowlisted_key() {
    let payload = payload_map("{}");
    let litellm_params =
        payload_map(r#"{"metadata": {"user_api_key_team_metadata": {"region": "west"}}}"#);
    let attributes = record_attributes(|span| {
        write_identity_attributes(span, &payload, &litellm_params, &["owner".to_owned()], &[]);
    });
    assert!(attributes.is_empty());
}

#[test]
fn identity_provider_model_prefers_hidden_params_and_falls_back_to_model() {
    let payload = payload_map(
        r#"{"hidden_params": {"litellm_model_name": "dispatched"}, "model": "fallback"}"#,
    );
    let attributes = record_attributes(|span| {
        write_identity_attributes(span, &payload, &BTreeMap::new(), &[], &[]);
    });
    assert_eq!(
        attributes,
        vec![(
            "litellm.provider.model".to_owned(),
            Value::from("dispatched")
        )]
    );
    let empty_hidden =
        payload_map(r#"{"hidden_params": {"litellm_model_name": ""}, "model": "fallback"}"#);
    let fallback = record_attributes(|span| {
        write_identity_attributes(span, &empty_hidden, &BTreeMap::new(), &[], &[]);
    });
    assert_eq!(
        fallback,
        vec![("litellm.provider.model".to_owned(), Value::from("fallback"))]
    );
}

#[test]
fn identity_skips_empty_route_and_model_group() {
    let payload =
        payload_map(r#"{"metadata": {"user_api_key_request_route": ""}, "model_group": ""}"#);
    let attributes = record_attributes(|span| {
        write_identity_attributes(span, &payload, &BTreeMap::new(), &[], &[]);
    });
    assert!(attributes.is_empty());
}

#[test]
fn service_tier_writes_requested_and_served_independently() {
    let payload = payload_map(
        r#"{"model_parameters": {"service_tier": "flex"}, "response": {"service_tier": "default"}}"#,
    );
    let attributes = record_attributes(|span| {
        set_service_tier_attributes(span, &payload);
    });
    assert_eq!(
        attributes,
        vec![
            (REQUESTED_TIER_ATTRIBUTE.to_owned(), Value::from("flex")),
            (SERVED_TIER_ATTRIBUTE.to_owned(), Value::from("default")),
        ]
    );
}

#[test]
fn service_tier_reads_a_served_tier_from_the_usage_object() {
    let payload = payload_map(r#"{"metadata": {"usage_object": {"service_tier": "priority"}}}"#);
    let attributes = record_attributes(|span| {
        set_service_tier_attributes(span, &payload);
    });
    assert_eq!(
        attributes,
        vec![(SERVED_TIER_ATTRIBUTE.to_owned(), Value::from("priority"))]
    );
}

#[test]
fn service_tier_skips_an_empty_served_tier_in_favor_of_the_usage_tier() {
    let payload = payload_map(
        r#"{"response": {"service_tier": ""}, "metadata": {"usage_object": {"service_tier": "batch"}}}"#,
    );
    let attributes = record_attributes(|span| {
        set_service_tier_attributes(span, &payload);
    });
    assert_eq!(
        attributes,
        vec![(SERVED_TIER_ATTRIBUTE.to_owned(), Value::from("batch"))]
    );
}

#[rstest]
#[case::unknown_requested_tier(r#"{"model_parameters": {"service_tier": "turbo"}}"#)]
#[case::missing_everything("{}")]
#[case::null_response(r#"{"response": null, "model_parameters": null}"#)]
fn service_tier_writes_nothing_without_a_known_tier(#[case] raw: &str) {
    let payload = payload_map(raw);
    let attributes = record_attributes(|span| {
        set_service_tier_attributes(span, &payload);
    });
    assert!(attributes.is_empty());
}
