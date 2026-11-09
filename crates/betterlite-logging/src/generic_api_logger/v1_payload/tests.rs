#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::{V1Kwargs, V1PayloadError, v1_logging_payload};

const RESPONSE: &str = r#"{"id":"chatcmpl-1","usage":{"total_tokens":5}}"#;
fn field<'payload>(
    payload: &'payload IndexMap<String, String>,
    name: &str,
) -> Option<&'payload str> {
    payload.get(name).map(String::as_str)
}
fn built(kwargs: &V1Kwargs, response: &Json) -> IndexMap<String, String> {
    v1_logging_payload(
        kwargs,
        response,
        Decimal::ZERO,
        Decimal::ZERO,
        "generated-id",
    )
    .expect("a payload")
}
fn full() -> V1Kwargs {
    let parameters = IndexMap::from([("temperature".to_owned(), Json::Text("0".to_owned()))]);
    V1Kwargs {
        call_type: Some("acompletion".to_owned()),
        cache_hit: Some(true),
        model: Some("gpt-4o".to_owned()),
        user: Some("user-1".to_owned()),
        model_parameters: Some(parameters),
        messages: Some(Json::Text("hi".to_owned())),
        metadata: Some(IndexMap::from([(
            "team".to_owned(),
            Json::Text("a".to_owned()),
        )])),
        cost: Some(Decimal::new(5, 1)),
    }
}
#[test]
fn v1_payload_has_the_thirteen_text_keys() {
    let payload = v1_logging_payload(
        &full(),
        &serde_json::from_str(RESPONSE).expect("a response"),
        Decimal::from(10_i64),
        Decimal::new(15, 1),
        "generated-id",
    )
    .expect("a payload");
    assert_eq!(payload.len(), 13);
    assert_eq!(field(&payload, "id"), Some("chatcmpl-1"));
    assert_eq!(field(&payload, "call_type"), Some("acompletion"));
    assert_eq!(field(&payload, "cache_hit"), Some("true"));
    assert_eq!(
        field(&payload, "startTime"),
        Some("1970-01-01T00:00:10+00:00")
    );
    assert_eq!(
        field(&payload, "endTime"),
        Some("1970-01-01T00:00:01.500+00:00")
    );
    assert_eq!(
        field(&payload, "modelParameters"),
        Some(r#"{"temperature":"0"}"#)
    );
    assert_eq!(field(&payload, "messages"), Some("hi"));
    assert_eq!(field(&payload, "model"), Some("gpt-4o"));
    assert_eq!(field(&payload, "user"), Some("user-1"));
    assert_eq!(field(&payload, "response"), Some(RESPONSE));
    assert_eq!(field(&payload, "metadata"), Some(r#"{"team":"a"}"#));
    assert_eq!(field(&payload, "usage"), Some(r#"{"total_tokens":5}"#));
    assert_eq!(field(&payload, "cost"), Some("0.5"));
}
#[test]
fn v1_payload_applies_the_defaults_and_fails_without_usage() {
    let without_id: Json = serde_json::from_str(r#"{"usage":{}}"#).expect("a response");
    let defaults = built(&V1Kwargs::default(), &without_id);
    assert_eq!(field(&defaults, "id"), Some("generated-id"));
    assert_eq!(field(&defaults, "call_type"), Some("litellm.completion"));
    assert_eq!(field(&defaults, "cache_hit"), Some("false"));
    assert_eq!(field(&defaults, "model"), Some(""));
    assert_eq!(field(&defaults, "user"), Some(""));
    assert_eq!(field(&defaults, "metadata"), Some("{}"));
    assert_eq!(field(&defaults, "modelParameters"), Some("{}"));
    assert_eq!(field(&defaults, "messages"), Some("null"));
    assert_eq!(field(&defaults, "cost"), Some("0"));
    let without: Json = serde_json::from_str(r#"{"id":"chatcmpl-1"}"#).expect("a response");
    let missing = v1_logging_payload(
        &V1Kwargs::default(),
        &without,
        Decimal::ZERO,
        Decimal::ZERO,
        "generated-id",
    );
    assert!(matches!(missing, Err(V1PayloadError::MissingUsage)));
}
