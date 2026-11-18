#![cfg(test)]

use super::{inputs, text_option};
use crate::sagemaker_runtime::error::RuntimeError;
use betterlite_models::decisions::Json;
use betterlite_openai::sagemaker_completion::params::Options;
use indexmap::IndexMap;
use serde_json::json;

#[test]
fn reads_prompt_text_and_uses_empty_for_non_text() {
    let body = IndexMap::from([
        (String::from("inputs"), Json::Text(String::from("prompt"))),
        (String::from("other"), Json::Null),
    ]);
    assert_eq!(inputs(&body), "prompt");
    let null = IndexMap::from([(String::from("inputs"), Json::Null)]);
    assert_eq!(inputs(&null), "");
    assert_eq!(inputs(&IndexMap::new()), "");
}

#[test]
fn reads_optional_text_and_rejects_a_wrong_type() {
    let mut params = Options::new();
    assert_eq!(text_option(&params, "model_id").expect("absent"), None);
    params.insert(String::from("model_id"), json!(null));
    assert_eq!(text_option(&params, "model_id").expect("null"), None);
    params.insert(String::from("model_id"), json!("blue"));
    assert_eq!(
        text_option(&params, "model_id").expect("string"),
        Some("blue")
    );
    params.insert(String::from("model_id"), json!(true));
    assert!(matches!(
        text_option(&params, "model_id"),
        Err(RuntimeError::Field { field: "model_id" })
    ));
}
