#![cfg(test)]

use betterlite_models::decisions::Json;
use serde_json::Number;

use super::masking::apply_masking_function;
use super::support::{capture_logs, value};

fn mask(data: &Json) -> Result<Json, &'static str> {
    match data {
        Json::Text(text) if text == "secret" => Err("the masking service refused the text"),
        Json::Text(text) => Ok(Json::Text(text.to_uppercase())),
        Json::Null => Ok(Json::Null),
        Json::Bool(_) => Ok(Json::Bool(false)),
        Json::Number(_) => Ok(Json::Number(Number::from(0))),
        Json::List(_) | Json::Object(_) => Ok(data.clone()),
    }
}

#[test]
fn mask_string_applies_the_function_to_text() {
    assert_eq!(
        apply_masking_function(&Json::Text("hello".to_owned()), &mask),
        Json::Text("HELLO".to_owned())
    );
}

#[test]
fn mask_dict_masks_every_nested_value() {
    let data =
        value(r#"{"message": "hello", "nested": {"text": "world"}, "count": 2, "flag": true}"#);
    let masked = apply_masking_function(&data, &mask);
    assert_eq!(
        masked,
        value(r#"{"message": "HELLO", "nested": {"text": "WORLD"}, "count": 0, "flag": false}"#)
    );
}

#[test]
fn mask_list_masks_each_item() {
    let data = value(r#"["hello", {"text": "world"}, 3, true]"#);
    let masked = apply_masking_function(&data, &mask);
    assert_eq!(masked, value(r#"["HELLO", {"text": "WORLD"}, 0, false]"#));
}

#[test]
fn mask_exception_logs_and_returns_the_original_data() {
    let (logs, _guard) = capture_logs();
    let data = value(r#"{"message": "secret", "other": "hello"}"#);
    let masked = apply_masking_function(&data, &mask);
    assert_eq!(masked, value(r#"{"message": "secret", "other": "HELLO"}"#));
    let text = logs.text();
    assert!(text.contains("Failed to apply masking function"), "{text}");
    assert!(text.contains("Returning original data."), "{text}");
}

#[test]
fn mask_null_stays_null() {
    let (logs, _guard) = capture_logs();
    assert_eq!(apply_masking_function(&Json::Null, &mask), Json::Null);
    assert!(!logs.text().contains("Failed to apply masking function"));
}
