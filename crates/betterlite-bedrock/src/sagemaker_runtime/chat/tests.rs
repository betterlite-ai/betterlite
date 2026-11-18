#![cfg(test)]

use super::text_field;
use crate::sagemaker_runtime::error::RuntimeError;
use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;

#[test]
fn reads_base_url_as_optional_text() {
    let absent = serde_json::from_str("{}").expect("empty body");
    assert_eq!(
        text_field(&absent, "sagemaker_base_url").expect("absent"),
        None
    );
    let null = serde_json::from_str(r#"{"sagemaker_base_url":null}"#).expect("null body");
    assert_eq!(text_field(&null, "sagemaker_base_url").expect("null"), None);
    let text = serde_json::from_str(r#"{"sagemaker_base_url":"https://example.test"}"#)
        .expect("text body");
    assert_eq!(
        text_field(&text, "sagemaker_base_url").expect("text"),
        Some("https://example.test")
    );
    let wrong = IndexMap::from([(String::from("sagemaker_base_url"), Json::Bool(true))]);
    assert!(matches!(
        text_field(&wrong, "sagemaker_base_url"),
        Err(RuntimeError::Field {
            field: "sagemaker_base_url"
        })
    ));
}
