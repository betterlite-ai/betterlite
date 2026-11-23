#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_core::google_genai::streaming::GenAiStreamError;
use betterlite_error::error::details::{RateLimitDetails, ResponseDetails};
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_error::rate_limit::RateLimitCategory;
use betterlite_models::decisions::Json as DecisionJson;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;

use super::{
    ConvertError, GenaiCall, NativeErrorPayload, decision_json, decision_object, parse,
    reject_files,
};

fn context(message: &str, model: &str, provider: &str) -> ErrorContext {
    ErrorContext {
        message: message.to_owned(),
        model: Some(model.to_owned()),
        provider: Some(provider.to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }
}

const FULL_CALL: &str = r#"{
    "model": "gemini/gemini-2.5-flash",
    "contents": [{"role": "user", "parts": [{"text": "Hi"}]}],
    "config": {"temperature": 1, "topK": 3},
    "tools": [{"functionDeclarations": [{"name": "f"}]}],
    "extra_headers": {"x-test": "1"},
    "extra_query": {"key": "value"},
    "extra_body": {"labels": {"a": "b"}},
    "timeout": 2.5,
    "custom_llm_provider": "gemini",
    "kwargs": {"api_key": "key"}
}"#;

#[test]
fn request_arguments_parse_field_for_field() {
    let call: GenaiCall = parse(FULL_CALL).unwrap();
    assert_eq!(call.model, "gemini/gemini-2.5-flash");
    let parts = Json::List(vec![Json::Object(Object::from([(
        "text".to_owned(),
        Json::Text("Hi".to_owned()),
    )]))]);
    let content = Json::Object(Object::from([
        ("role".to_owned(), Json::Text("user".to_owned())),
        ("parts".to_owned(), parts),
    ]));
    assert_eq!(call.contents, Json::List(vec![content]));
    assert_eq!(
        call.config,
        Some(Object::from([
            (
                "temperature".to_owned(),
                Json::Number(serde_json::Number::from(1_u8))
            ),
            (
                "topK".to_owned(),
                Json::Number(serde_json::Number::from(3_u8))
            ),
        ]))
    );
}

#[test]
fn request_extras_parse_field_for_field() {
    let call: GenaiCall = parse(FULL_CALL).unwrap();
    assert_eq!(
        call.extra_headers,
        Some(IndexMap::from([("x-test".to_owned(), "1".to_owned())]))
    );
    assert_eq!(
        call.extra_query,
        Some(Object::from([(
            "key".to_owned(),
            Json::Text("value".to_owned())
        )]))
    );
    assert_eq!(
        call.extra_body,
        Some(Object::from([(
            "labels".to_owned(),
            Json::Object(Object::from([("a".to_owned(), Json::Text("b".to_owned()))]))
        )]))
    );
    assert_eq!(call.timeout().unwrap(), Some(Duration::from_millis(2500)));
    assert_eq!(call.custom_llm_provider.as_deref(), Some("gemini"));
    assert_eq!(
        call.kwargs.get("api_key"),
        Some(&Json::Text("key".to_owned()))
    );
    assert!(call.tools.is_some());
}

#[test]
fn request_absent_optional_fields_stay_none() {
    let call: GenaiCall = parse(r#"{"model": "m", "contents": [], "kwargs": {}}"#).unwrap();
    assert_eq!(call.config, None);
    assert_eq!(call.tools, None);
    assert_eq!(call.extra_headers, None);
    assert_eq!(call.extra_query, None);
    assert_eq!(call.extra_body, None);
    assert_eq!(call.custom_llm_provider, None);
    assert_eq!(call.timeout().unwrap(), None);
}

#[test]
fn request_negative_timeout_is_refused() {
    let call: GenaiCall =
        parse(r#"{"model": "m", "contents": [], "timeout": -1, "kwargs": {}}"#).unwrap();
    assert!(matches!(call.timeout(), Err(ConvertError::Timeout { .. })));
}

#[test]
fn request_oversized_timeout_is_refused() {
    let call: GenaiCall =
        parse(r#"{"model": "m", "contents": [], "timeout": 1e300, "kwargs": {}}"#).unwrap();
    assert!(matches!(call.timeout(), Err(ConvertError::Timeout { .. })));
}

#[test]
fn request_zero_timeout_is_a_zero_duration() {
    let call: GenaiCall =
        parse(r#"{"model": "m", "contents": [], "timeout": 0, "kwargs": {}}"#).unwrap();
    assert_eq!(call.timeout().unwrap(), Some(Duration::ZERO));
}

#[test]
fn request_file_marker_is_refused() {
    let text = r#"{"contents": [{"parts": [{"$file": {"blob": 0, "name": "a.txt"}}]}]}"#;
    let mut blobs = vec![b"hi".to_vec()];
    assert!(matches!(
        reject_files(text, &mut blobs),
        Err(ConvertError::Files)
    ));
}

#[test]
fn request_payload_without_files_passes() {
    let text = r#"{"contents": [{"parts": [{"text": "Hi"}]}]}"#;
    let mut blobs: Vec<Vec<u8>> = Vec::new();
    reject_files(text, &mut blobs).unwrap();
}

#[test]
fn request_decision_json_keeps_every_value() {
    let text = r#"{"a": null, "b": true, "c": 1, "d": "x", "e": [false]}"#;
    let value: Json = serde_json::from_str(text).unwrap();
    let converted = decision_json(&value);
    assert_eq!(
        converted,
        DecisionJson::Object(IndexMap::from([
            ("a".to_owned(), DecisionJson::Null),
            ("b".to_owned(), DecisionJson::Bool(true)),
            (
                "c".to_owned(),
                DecisionJson::Number(serde_json::Number::from(1_u8))
            ),
            ("d".to_owned(), DecisionJson::Text("x".to_owned())),
            (
                "e".to_owned(),
                DecisionJson::List(vec![DecisionJson::Bool(false)])
            ),
        ]))
    );
}

#[test]
fn request_decision_object_keeps_every_key() {
    let object = Object::from([
        ("a".to_owned(), Json::Null),
        ("b".to_owned(), Json::Text("x".to_owned())),
    ]);
    let converted = decision_object(&object);
    assert_eq!(converted.len(), 2);
    assert_eq!(
        converted.get("b"),
        Some(&DecisionJson::Text("x".to_owned()))
    );
}

#[test]
fn error_payload_names_the_litellm_class() {
    let class = ErrorClass::BadRequest(Box::new(ResponseDetails::<400>::from(context(
        "API key not valid",
        "gemini/gemini-2.5-flash",
        "gemini",
    ))));
    let payload = NativeErrorPayload::from_class(&class);
    assert_eq!(payload.kind, "BadRequestError");
    assert_eq!(payload.message, "API key not valid");
    assert_eq!(payload.status_code, Some(400));
    assert_eq!(payload.llm_provider.as_deref(), Some("gemini"));
    assert_eq!(payload.model.as_deref(), Some("gemini/gemini-2.5-flash"));
    assert_eq!(payload.headers, BTreeMap::new());
    assert_eq!(payload.max_retries, None);
}

#[test]
fn error_payload_keeps_rate_limit_headers() {
    let class = ErrorClass::RateLimit(Box::new(RateLimitDetails {
        context: context("slow down", "gemini/g", "gemini"),
        category: RateLimitCategory::VendorRateLimit,
        dimension: None,
        headers: BTreeMap::from([("retry-after".to_owned(), "5".to_owned())]),
    }));
    let payload = NativeErrorPayload::from_class(&class);
    assert_eq!(payload.kind, "RateLimitError");
    assert_eq!(
        payload.headers.get("retry-after").map(String::as_str),
        Some("5")
    );
}

#[test]
fn error_payload_from_stream_is_a_plain_exception() {
    let payload = NativeErrorPayload::from_stream(&GenAiStreamError::EventTooLong { limit: 16 });
    assert_eq!(payload.kind, "Exception");
    assert_eq!(payload.status_code, None);
    assert_eq!(payload.headers, BTreeMap::new());
    assert!(payload.message.contains("16"));
}

#[test]
fn error_payload_encodes_as_json() {
    let class = ErrorClass::BadRequest(Box::new(ResponseDetails::<400>::from(context(
        "bad", "gemini/g", "gemini",
    ))));
    let text = serde_json::to_string(&NativeErrorPayload::from_class(&class)).unwrap();
    assert!(text.contains(r#""type":"BadRequestError""#));
    assert!(text.contains(r#""status_code":400"#));
}
