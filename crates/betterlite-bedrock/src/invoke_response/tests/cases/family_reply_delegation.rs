#![cfg(test)]

use betterlite_models::decisions::Json;

use crate::invoke_response::error::InvokeReplyError;
use crate::invoke_response::tests::support::{decode, fixture, headers, json};

const CLAUDE_MODEL: &str = "anthropic.claude-3-5-sonnet-20240620-v1:0";
const NOVA_MODEL: &str = "amazon.nova-pro-v1:0";
const PEGASUS_MODEL: &str = "twelvelabs.pegasus-1-2-v1:0";
const CLAUDE_REPLY: &str = r#"{
    "id": "msg_bdrk_01",
    "model": "claude-plain",
    "type": "message",
    "role": "assistant",
    "content": [{"type": "text", "text": "hi there"}],
    "stop_reason": "end_turn",
    "usage": {
        "input_tokens": 10,
        "output_tokens": 5,
        "cache_creation_input_tokens": 2,
        "cache_read_input_tokens": 3
    }
}"#;
const NOVA_REPLY: &str = r#"{"output": {"message": {"role": "assistant", "content": [{"text": "done"}]}}, "stopReason": "end_turn", "usage": {"inputTokens": 10, "outputTokens": 5, "totalTokens": 15}}"#;
const PREFIX_REPLY: &str = r#"{"id": "msg_prefix", "model": "claude-plain", "type": "message", "role": "assistant", "content": [{"type": "text", "text": "fix"}], "stop_reason": "end_turn", "usage": {"input_tokens": 1, "output_tokens": 1}}"#;
const CLAUDE_JSON_REPLY: &str = r#"{"id": "msg_bdrk_json", "model": "claude-json", "type": "message", "role": "assistant", "content": [{"type": "tool_use", "id": "toolu_01", "name": "json_tool_call", "input": {"answer": 42}}], "stop_reason": "tool_use", "usage": {"input_tokens": 10, "output_tokens": 5}}"#;
const NOVA_JSON_REPLY: &str = r#"{"output": {"message": {"role": "assistant", "content": [{"toolUse": {"toolUseId": "toolu_02", "name": "json_tool_call", "input": {"answer": 42}}}]}}, "stopReason": "tool_use", "usage": {"inputTokens": 10, "outputTokens": 5, "totalTokens": 15}}"#;

pub(in crate::invoke_response::tests) fn family_reply_delegation() {
    anthropic_reply();
    nova_reply();
    pegasus_reply();
    delegated_errors_become_processing_errors();
    request_messages_feed_the_prefix_prompt();
    invalid_response_header_names_are_rejected();
    repeated_response_headers_stay_duplicate();
    anthropic_json_mode_reads_the_tool_arguments();
    nova_json_mode_reads_the_tool_arguments();
}

fn anthropic_reply() {
    let mut case = fixture();
    case.model = CLAUDE_MODEL.to_owned();
    let run = decode(&case, CLAUDE_REPLY);
    assert!(run.calls.is_empty(), "the delegate owns usage");
    let envelope = run.envelope();
    assert_eq!(envelope.id, "chatcmpl-supplied");
    assert_eq!(envelope.model.as_deref(), Some("claude-plain"));
    assert_eq!(envelope.created, 1_700_000_000);
    assert_eq!(
        envelope.choices[0].message.content.as_deref(),
        Some("hi there")
    );
    assert_eq!(envelope.choices[0].finish_reason.as_deref(), Some("stop"));
    let usage = envelope.usage.as_ref().expect("the delegate sets usage");
    assert_eq!(usage.prompt_tokens, 15);
    assert_eq!(usage.completion_tokens, 5);
    assert_eq!(usage.total_tokens, 20);
}

fn nova_reply() {
    let mut case = fixture();
    case.model = NOVA_MODEL.to_owned();
    let run = decode(&case, NOVA_REPLY);
    assert!(run.calls.is_empty(), "the delegate owns usage");
    let reply = run.nova();
    assert_eq!(reply.choices[0].message.content.as_deref(), Some("done"));
    assert_eq!(reply.created, 1_700_000_000);
    assert_eq!(reply.model, case.model);
    let usage = reply.usage.as_ref().expect("the delegate sets usage");
    assert_eq!(usage.prompt_tokens, 10);
    assert_eq!(usage.completion_tokens, 5);
    assert_eq!(usage.total_tokens, 15);
}

fn pegasus_reply() {
    let mut case = fixture();
    case.model = PEGASUS_MODEL.to_owned();
    case.headers = headers("3", "2");
    let run = decode(
        &case,
        r#"{"message": "A cat walks.", "finishReason": "length"}"#,
    );
    assert!(run.calls.is_empty(), "the headers own the usage");
    let reply = run.pegasus();
    assert_eq!(
        reply.choices[0].message.content.as_deref(),
        Some("A cat walks.")
    );
    assert_eq!(reply.choices[0].finish_reason.as_deref(), Some("length"));
    assert_eq!(reply.created, 1_700_000_000);
    assert_eq!(reply.model, case.model);
    assert_eq!(reply.usage.prompt_tokens, 3);
    assert_eq!(reply.usage.completion_tokens, 2);
    assert_eq!(reply.usage.total_tokens, 5);
}

fn delegated_errors_become_processing_errors() {
    for (model, body) in [
        (CLAUDE_MODEL, r#"{"id": "msg_1", "type": "message"}"#),
        (NOVA_MODEL, r#"{"output": {}}"#),
        (PEGASUS_MODEL, "{}"),
    ] {
        let mut case = fixture();
        case.model = model.to_owned();
        case.headers = vec![(String::from("x-amzn-requestid"), String::from("req-1"))];
        let run = decode(&case, body);
        assert_eq!(run.status(), Some(422), "{model}");
        assert!(
            run.message()
                .starts_with(&format!("Error processing={body}, Received error=")),
            "{model}: {}",
            run.message()
        );
        assert_eq!(run.response_headers(), case.headers.as_slice(), "{model}");
    }
}

fn request_messages_feed_the_prefix_prompt() {
    let mut case = fixture();
    case.model = CLAUDE_MODEL.to_owned();
    case.messages = json(r#"[{"role": "assistant", "content": "pre", "prefix": true}]"#);
    let run = decode(&case, PREFIX_REPLY);
    assert_eq!(
        run.envelope().choices[0].message.content.as_deref(),
        Some("prefix")
    );
}

fn invalid_response_header_names_are_rejected() {
    let mut case = fixture();
    case.model = CLAUDE_MODEL.to_owned();
    case.headers = vec![(String::from("bad header"), String::from("x"))];
    let run = decode(&case, CLAUDE_REPLY);
    assert!(matches!(
        run.failure(),
        InvokeReplyError::InvalidHeader { name } if name == "bad header"
    ));
}

fn anthropic_json_mode_reads_the_tool_arguments() {
    let mut case = fixture();
    case.model = CLAUDE_MODEL.to_owned();
    case.json_mode = Some(true);
    let run = decode(&case, CLAUDE_JSON_REPLY);
    let envelope = run.envelope();
    assert_eq!(
        envelope.choices[0].message.content.as_deref(),
        Some(r#"{"answer": 42}"#)
    );
    assert!(envelope.choices[0].message.tool_calls.is_none());
    assert_eq!(envelope.choices[0].finish_reason.as_deref(), Some("stop"));
}

fn nova_json_mode_reads_the_tool_arguments() {
    let mut case = fixture();
    case.model = NOVA_MODEL.to_owned();
    case.optional.json_mode = Some(true);
    let run = decode(&case, NOVA_JSON_REPLY);
    let reply = run.nova();
    assert_eq!(
        reply.choices[0].message.content.as_deref(),
        Some(r#"{"answer": 42}"#)
    );
    assert!(reply.choices[0].message.tool_calls.is_none());
    assert_eq!(reply.choices[0].finish_reason.as_str(), "stop");
}

// httpx keeps a repeated response header as two values, and the Anthropic header reader joins them
fn repeated_response_headers_stay_duplicate() {
    let mut case = fixture();
    case.model = CLAUDE_MODEL.to_owned();
    case.headers = vec![
        (String::from("x-amzn-requestid"), String::from("first")),
        (String::from("x-amzn-requestid"), String::from("second")),
    ];
    let run = decode(&case, CLAUDE_REPLY);
    let Some(Json::Object(additional)) = run.envelope().hidden_params.get("additional_headers")
    else {
        panic!("the additional headers are an object");
    };
    assert_eq!(
        additional.get("llm_provider-x-amzn-requestid"),
        Some(&Json::Text(String::from("first, second")))
    );
}
