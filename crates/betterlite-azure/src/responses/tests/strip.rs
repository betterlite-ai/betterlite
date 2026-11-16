use betterlite_openai::response_input::ResponsesInput;
use betterlite_prompts::arguments::json::Object;

use super::support;
use crate::responses::error::ResponsesError;
use crate::responses::request::{strip_model, transform_request};

fn text_input() -> ResponsesInput {
    ResponsesInput::Text(String::from("hello"))
}

fn transformed(model: &str, options: &str) -> Object {
    transform_request(model, &text_input(), &support::object(options), None).unwrap()
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
    assert_eq!(body["model"], support::json(r#""gpt-5""#));
    assert_eq!(body["temperature"], support::json("0.2"));
    assert_eq!(body["input"], support::json(r#""hello""#));
    flattened_tools();
}

fn flattened_tools() {
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
        support::json(
            r#"[
            {"type":"function","name":"f","description":"inner","parameters":{"type":"object"}},
            {"type":"web_search"},
            "not-a-tool"
        ]"#
        )
    );
    // a nested function that is not an object cannot be merged
    let merged = transform_request(
        "gpt-5",
        &text_input(),
        &support::object(r#"{"tools":[{"function":"name"}]}"#),
        None,
    );
    assert!(matches!(merged, Err(ResponsesError::InvalidToolFunction)));
    // a tools value that is not a list is left to the shared request transform
    let unlisted = transform_request(
        "gpt-5",
        &text_input(),
        &support::object(r#"{"tools":{}}"#),
        None,
    );
    assert!(matches!(unlisted, Err(ResponsesError::Input(_))));
}
