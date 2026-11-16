use betterlite_openai::openai_base_config::{Headers, Object};

use crate::chat::FoundryChatOptions;
use crate::chat::response::{ResponseError, transform_response};

use super::support::{EnvironmentFixture, NoSources, body, settings};

const COMPLETE: &str = r#"{
  "id": "chatcmpl-fixture",
  "object": "chat.completion",
  "created": 1700000000,
  "model": "provider-sent",
  "choices": [{"index": 0, "message": {"role": "assistant", "content": "hi"}, "finish_reason": "stop"}],
  "usage": {"prompt_tokens": 3, "completion_tokens": 4, "total_tokens": 7},
  "system_fingerprint": "fp-fixture",
  "provider_extra": {"kept": true}
}"#;

const WITHOUT_MODEL: &str = r#"{
  "id": "chatcmpl-fixture",
  "object": "chat.completion",
  "created": 1700000000,
  "choices": [{"index": 0, "message": {"role": "assistant", "content": "hi"}}]
}"#;

pub(super) fn azure_ai_response_model() {
    let response = transform_response("deployment", body(COMPLETE)).expect("the body parses");
    assert_eq!(response.model, "azure_ai/provider-sent");
    assert_eq!(response.id, "chatcmpl-fixture");
    assert_eq!(response.object, "chat.completion");
    assert_eq!(response.created, 1_700_000_000);
    assert_eq!(response.system_fingerprint.as_deref(), Some("fp-fixture"));
    assert_eq!(response.usage.total_tokens, 7);
    assert_eq!(response.choices.len(), 1);
    assert_eq!(response.choices[0].message.content.as_deref(), Some("hi"));
    assert_eq!(response.choices[0].finish_reason.as_deref(), Some("stop"));
    defaults_and_refusals();
    refuses_the_wrong_shape();
}

fn defaults_and_refusals() {
    let missing = transform_response("deployment", body(WITHOUT_MODEL))
        .expect("a body without a model keeps the assigned one");
    assert_eq!(missing.model, "azure_ai/deployment");
    assert_eq!(
        missing.usage,
        betterlite_openai::chat::response::ChatUsage::default()
    );
    let null = transform_response(
        "deployment",
        body(
            r#"{"id":"i","object":"chat.completion","created":1,"model":null,
                "choices":[{"index":0,"message":{"role":"assistant","content":"hi"}}]}"#,
        ),
    )
    .expect("a null model keeps the assigned one");
    assert_eq!(null.model, "azure_ai/deployment");
    let nested = transform_response(
        "deployment",
        body(
            r#"{"id":"i","object":"chat.completion","created":1,"model":"azure_ai/inner",
                "choices":[{"index":0,"message":{"role":"assistant","content":"hi"}}]}"#,
        ),
    )
    .expect("the body model is prefixed as sent");
    assert_eq!(nested.model, "azure_ai/azure_ai/inner");
}

fn refuses_the_wrong_shape() {
    let numeric = transform_response(
        "deployment",
        body(
            r#"{"id":"i","object":"chat.completion","created":1,"model":7,
                "choices":[{"index":0,"message":{"role":"assistant","content":"hi"}}]}"#,
        ),
    )
    .expect_err("a numeric model is refused");
    assert!(matches!(numeric, ResponseError::ModelNotString));
    let list = transform_response("deployment", body("[]")).expect_err("a list is not a body");
    assert!(matches!(list, ResponseError::NotAnObject));
    let incomplete = transform_response(
        "deployment",
        body(r#"{"object":"chat.completion","created":1,"model":"m","choices":[]}"#),
    )
    .expect_err("the response shape needs an id");
    assert!(matches!(incomplete, ResponseError::Json(_)));
}

#[test]
fn parse_stages_the_response_through_the_body_transform() {
    let optional = Object::new();
    let credentials = settings(None, None);
    let options = FoundryChatOptions {
        model: "deployment",
        api_version: None,
        messages: &[],
        optional_params: &optional,
        environment: EnvironmentFixture {
            headers: Headers::new(),
            argument_key: None,
            params_key: None,
            configured_key: None,
            credentials: &credentials,
            sources: &NoSources,
            refresh_enabled: false,
            api_base: Some("https://studio.example.test"),
        }
        .request(),
    };
    let parsed = options.parse(body(COMPLETE)).expect("the body parses");
    assert_eq!(parsed.model, "azure_ai/provider-sent");
    assert_eq!(parsed.choices.len(), 1);
}
