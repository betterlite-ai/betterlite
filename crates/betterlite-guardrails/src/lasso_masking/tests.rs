#![cfg(test)]

use serde_json::json;

use super::{apply_model_response, expand_for_classification, map_masked_messages_back};

#[test]
fn lasso_masking_behaviors() {
    let messages = vec![
        json!({"role":"assistant","content":null,"tool_calls":[{"id":"call","function":{"name":"lookup","arguments":"{\"email\":\"a\"}"}}]}),
    ];
    let expanded = expand_for_classification(&messages);
    assert_eq!(expanded[0]["content"]["type"], "tool_use");
    let masked = vec![
        json!({"role":"model","content":{"type":"tool_use","id":"call","name":"lookup","input":{"email":"[MASKED EMAIL]"}}}),
    ];
    assert_eq!(
        map_masked_messages_back(&messages, &masked)[0]["tool_calls"][0]["function"]["arguments"],
        "{\"email\":\"[MASKED EMAIL]\"}"
    );
}

#[test]
fn lasso_masking_lifecycle() {
    let original = vec![json!({"role":"user","content":"secret"})];
    let masked = vec![json!({"role":"user","content":"[MASKED]"})];
    assert_eq!(
        map_masked_messages_back(&original, &masked)[0]["content"],
        "[MASKED]"
    );
}

#[test]
fn lasso_masking_response_rewrites_text_and_tool_arguments() {
    let mut response = json!({
        "choices": [{"message": {
            "role": "assistant",
            "content": "secret",
            "tool_calls": [{"id": "call", "function": {"arguments": "{}"}}]
        }}]
    });
    let masked = vec![
        json!({"role":"assistant","content":"[MASKED]"}),
        json!({"role":"model","content":{"type":"tool_use","id":"call","input":{"email":"[MASKED EMAIL]"}}}),
    ];
    apply_model_response(&mut response, &masked);
    assert_eq!(response["choices"][0]["message"]["content"], "[MASKED]");
    assert_eq!(
        response["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"],
        "{\"email\":\"[MASKED EMAIL]\"}"
    );
}
