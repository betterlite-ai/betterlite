#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::{V1Kwargs, V1PayloadError, v1_logging_payload};

fn response() -> Json {
    serde_json::from_str(r#"{"id":"chatcmpl-1","usage":{"total_tokens":5}}"#).expect("a response")
}

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

#[test]
fn v1_payload_has_the_thirteen_text_keys() {
    let metadata = IndexMap::from([("team".to_owned(), Json::Text("a".to_owned()))]);
    let kwargs = V1Kwargs {
        call_type: Some("acompletion".to_owned()),
        cache_hit: Some(true),
        model: Some("gpt-4o".to_owned()),
        user: None,
        model_parameters: None,
        messages: None,
        metadata: Some(metadata),
        cost: Some(Decimal::new(5, 1)),
    };
    let payload = v1_logging_payload(
        &kwargs,
        &response(),
        Decimal::from(10_i64),
        Decimal::new(15, 1),
        "generated-id",
    )
    .expect("a payload");
    assert_eq!(payload.len(), 13);
    assert_eq!(field(&payload, "id"), Some("chatcmpl-1"));
    assert_eq!(field(&payload, "call_type"), Some("acompletion"));
    assert_eq!(field(&payload, "cache_hit"), Some("true"));
    assert_eq!(field(&payload, "metadata"), Some(r#"{"team":"a"}"#));
    assert_eq!(field(&payload, "usage"), Some(r#"{"total_tokens":5}"#));
    let without_id: Json = serde_json::from_str(r#"{"usage":{}}"#).expect("a response");
    let defaults = built(&V1Kwargs::default(), &without_id);
    assert_eq!(field(&defaults, "id"), Some("generated-id"));
}

#[test]
fn v1_payload_missing_usage_fails() {
    let without: Json = serde_json::from_str(r#"{"id":"chatcmpl-1"}"#).expect("a response");
    assert!(matches!(
        v1_logging_payload(
            &V1Kwargs::default(),
            &without,
            Decimal::ZERO,
            Decimal::ZERO,
            "generated-id"
        ),
        Err(V1PayloadError::MissingUsage)
    ));
}
