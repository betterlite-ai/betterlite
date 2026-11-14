use betterlite_openai::response_input::ResponsesInput;
use betterlite_prompts::arguments::json::Object;
use tracing::Level;

use crate::mantle_responses::error::MantleResponsesError;
use crate::mantle_responses::request::transform_responses_request;

use super::support;

const MODEL: &str = "us-east-1/openai.gpt-oss-120b";

const REWRITE_WARNING: &str = "Bedrock Mantle Responses API: rewrote Codex input item type(s) ['agent_message'] that Mantle rejects.";

pub(super) fn mantle_input_hoisting() {
    hoist_cases();
    empty_items_cases();
    rewrite_cases();
    filtered_cases();
    plain_cases();
    error_cases();
}

fn input(source: &str) -> ResponsesInput {
    serde_json::from_str(source).expect("fixture input")
}

fn body(model: &str, input_json: &str, params: &Object) -> Object {
    let request = input(input_json);
    transform_responses_request(model, &request, params, None).expect("the body builds")
}

fn hoist_cases() {
    let (messages, _guard) = support::capture();
    let params = support::object(r#"{"temperature": 0.5}"#);
    let request = input(
        r#"[{"type": "message", "role": "user", "content": "hi"}, {"type": "additional_tools", "tools": [{"type": "function", "name": "hoisted", "parameters": {}}]}]"#,
    );
    let built = transform_responses_request(MODEL, &request, &params, None).expect("the body");
    assert_eq!(
        built,
        support::object(
            r#"{"model": "us-east-1/openai.gpt-oss-120b", "temperature": 0.5, "input": [{"type": "message", "role": "user", "content": "hi"}], "tools": [{"type": "function", "name": "hoisted", "parameters": {}}]}"#
        )
    );
    assert!(messages.at(Level::WARN).is_empty());
}

fn empty_items_cases() {
    let (messages, _guard) = support::capture();
    let params = support::object(r#"{"temperature": 0.5}"#);
    let built = body(
        MODEL,
        r#"[{"type": "additional_tools"}, {"type": "message", "role": "user", "content": "hi"}]"#,
        &params,
    );
    assert_eq!(
        built,
        support::object(
            r#"{"model": "us-east-1/openai.gpt-oss-120b", "temperature": 0.5, "input": [{"type": "message", "role": "user", "content": "hi"}]}"#
        )
    );
    let empty_list = body(
        MODEL,
        r#"[{"type": "additional_tools", "tools": []}]"#,
        &params,
    );
    assert_eq!(
        empty_list,
        support::object(
            r#"{"model": "us-east-1/openai.gpt-oss-120b", "temperature": 0.5, "input": []}"#
        )
    );
    assert!(messages.at(Level::WARN).is_empty());
}

fn rewrite_cases() {
    let (messages, _guard) = support::capture();
    let params = support::object("{}");
    let built = body(
        MODEL,
        r#"[{"type": "agent_message", "content": [{"type": "output_text", "text": "hello"}]}]"#,
        &params,
    );
    assert_eq!(
        built,
        support::object(
            r#"{"model": "us-east-1/openai.gpt-oss-120b", "input": [{"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "hello"}]}]}"#
        )
    );
    assert_eq!(messages.at(Level::WARN), vec![REWRITE_WARNING.to_owned()]);
    let dropped = body(
        MODEL,
        r#"[{"type": "context_compaction"}, {"type": "agent_message", "content": [{"type": "output_text", "text": "hi"}]}]"#,
        &params,
    );
    assert_eq!(
        dropped,
        support::object(
            r#"{"model": "us-east-1/openai.gpt-oss-120b", "input": [{"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "hi"}]}]}"#
        )
    );
    assert_eq!(
        messages.at(Level::WARN),
        vec![
            REWRITE_WARNING.to_owned(),
            "Bedrock Mantle Responses API: rewrote Codex input item type(s) ['agent_message', 'context_compaction'] that Mantle rejects.".to_owned(),
        ]
    );
}

fn filtered_cases() {
    let (messages, _guard) = support::capture();
    let params = support::object(
        r#"{"tools": [{"type": "function", "name": "existing", "parameters": {}}]}"#,
    );
    let built = body(
        MODEL,
        r#"[{"type": "additional_tools", "tools": [{"type": "function", "name": "hoisted", "parameters": {}}, {"type": "web_search_preview"}]}]"#,
        &params,
    );
    assert_eq!(
        built,
        support::object(
            r#"{"model": "us-east-1/openai.gpt-oss-120b", "input": [], "tools": [{"type": "function", "name": "existing", "parameters": {}}, {"type": "function", "name": "hoisted", "parameters": {}}]}"#
        )
    );
    assert_eq!(
        messages.at(Level::WARN),
        vec![
            "Bedrock Mantle Responses API: dropping unsupported tool type(s) ['web_search_preview'] (supported: ['custom', 'function', 'mcp', 'namespace', 'tool_search', 'web_search']).".to_owned(),
        ]
    );
    let empty_params = support::object("{}");
    let removed = body(
        MODEL,
        r#"[{"type": "additional_tools", "tools": [{"type": "code_interpreter"}]}]"#,
        &empty_params,
    );
    assert_eq!(
        removed,
        support::object(r#"{"model": "us-east-1/openai.gpt-oss-120b", "input": []}"#)
    );
}

fn plain_cases() {
    let (messages, _guard) = support::capture();
    let params = support::object(r#"{"temperature": 0.5}"#);
    assert_eq!(
        body("openai.gpt-5.5", r#""hello""#, &params),
        support::object(r#"{"model": "openai.gpt-5.5", "temperature": 0.5, "input": "hello"}"#)
    );
    assert!(messages.at(Level::WARN).is_empty());
}

fn error_cases() {
    let (messages, _guard) = support::capture();
    let fixed = support::object(r#"{"model": "already-set"}"#);
    let request = input(r#""hello""#);
    let error = transform_responses_request(MODEL, &request, &fixed, None)
        .expect_err("the body cannot fix the model twice");
    assert!(matches!(&error, MantleResponsesError::Input(_)));
    assert_eq!(error.status_code(), None);
    let tools = support::object(r#"{"tools": {"type": "function"}}"#);
    let error = transform_responses_request(MODEL, &request, &tools, None)
        .expect_err("a non-list tools entry is refused");
    assert!(matches!(&error, MantleResponsesError::Input(_)));
    assert_eq!(error.status_code(), Some(400));
    assert!(messages.at(Level::WARN).is_empty());
}
