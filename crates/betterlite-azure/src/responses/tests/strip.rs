use betterlite_openai::response_input::ResponsesInput;
use serde_json::{Value, json};

use super::support;
use crate::responses::error::ResponsesError;
use crate::responses::request::{strip_model, transform_request};

fn text_input() -> ResponsesInput {
    ResponsesInput::Text(String::from("hello"))
}

fn transformed(model: &str, options: &str) -> Value {
    let body = transform_request(model, &text_input(), &support::object(options), None).unwrap();
    serde_json::to_value(body).unwrap()
}

pub(super) fn strip_azure_model_prefix() {
    assert_eq!(strip_model("responses/o_series/azure_ai/gpt-5"), "gpt-5");
    assert_eq!(strip_model("azure/gpt-5"), "azure/gpt-5");
    assert_eq!(strip_model("openai/gpt-5"), "openai/gpt-5");
    // every occurrence goes, wherever it sits
    assert_eq!(
        strip_model("deployments/responses/x/responses/y"),
        "deployments/x/y"
    );
    assert_eq!(
        strip_model("azure_ai/vendor/o_series/model"),
        "vendor/model"
    );
    // the request names the stripped model and keeps the other options
    let body = transformed("azure_ai/responses/gpt-5", r#"{"temperature":0.2}"#);
    assert_eq!(body["model"], json!("gpt-5"));
    assert_eq!(body["temperature"], json!(0.2));
    assert_eq!(body["input"], json!("hello"));
    flattened_tools();
}

pub(super) fn flattened_tools() {
    let body = transformed(
        "o_series/gpt-5",
        r#"{"tools":[
            {"type":"function","description":"outer","function":{"name":"f","description":"inner","parameters":{"type":"object"}}},
            {"type":"web_search"},
            "not-a-tool"
        ]}"#,
    );
    assert_eq!(
        body["tools"],
        json!([
            {"type":"function","name":"f","description":"inner","parameters":{"type":"object"}},
            {"type":"web_search"},
            "not-a-tool"
        ])
    );
    // a nested function that is not an object cannot be merged
    let error = transform_request(
        "gpt-5",
        &text_input(),
        &support::object(r#"{"tools":[{"function":"name"}]}"#),
        None,
    )
    .unwrap_err();
    assert!(matches!(error, ResponsesError::InvalidToolFunction));
    // a tools value that is not a list is left to the shared request transform
    let error = transform_request(
        "gpt-5",
        &text_input(),
        &support::object(r#"{"tools":{}}"#),
        None,
    )
    .unwrap_err();
    assert!(matches!(error, ResponsesError::Input(_)));
}
