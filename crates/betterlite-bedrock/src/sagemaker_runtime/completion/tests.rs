#![cfg(test)]

use super::{inputs, text_option, unreadable_reply};
use crate::sagemaker_runtime::error::RuntimeError;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Value;

#[test]
fn reads_prompt_text_and_uses_empty_for_non_text() {
    let body = IndexMap::from([
        (String::from("inputs"), Json::Text(String::from("prompt"))),
        (String::from("other"), Json::Null),
    ]);
    assert_eq!(inputs(&body), "prompt");
    let body = IndexMap::from([(String::from("inputs"), Json::Null)]);
    assert_eq!(inputs(&body), "");
    assert_eq!(inputs(&IndexMap::new()), "");
}

#[test]
fn reads_optional_text_and_rejects_a_wrong_type() {
    let mut params = IndexMap::new();
    assert_eq!(text_option(&params, "model_id").expect("absent"), None);
    params.insert(String::from("model_id"), Value::Null);
    assert_eq!(text_option(&params, "model_id").expect("null"), None);
    params.insert(
        String::from("model_id"),
        Value::String(String::from("blue")),
    );
    assert_eq!(
        text_option(&params, "model_id").expect("string"),
        Some("blue")
    );
    params.insert(String::from("model_id"), Value::Bool(true));
    assert!(matches!(
        text_option(&params, "model_id"),
        Err(RuntimeError::Field { field: "model_id" })
    ));
}

#[test]
fn malformed_reply_error_keeps_status_and_reply_text() {
    let body = b"not-json";
    let source = serde_json::from_slice::<Value>(body).expect_err("malformed JSON");
    let error = unreadable_reply(body, &source);
    assert_eq!(error.status_code, 500);
    assert!(error.message.contains("not-json"));
}
