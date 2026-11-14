use std::sync::Mutex;

use betterlite_prompts::arguments::json::Json;

use crate::snowflake::tests::support::{WordCounter, answer, parsed};

use super::{
    PostCall, PostCallLogger, ResponseInput, transform_response_claude, transform_response_openai,
};

const OPENAI_REPLY: &str = r#"{"id":"chatcmpl-1","object":"chat.completion","created":7,"model":"mistral-large","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#;

struct Recorded {
    input: Vec<Json>,
    api_key: String,
    request_data: Json,
    original_response: Json,
}

#[derive(Default)]
struct Recorder {
    calls: Mutex<Vec<Recorded>>,
}

impl PostCallLogger for Recorder {
    fn post_call(&self, call: PostCall<'_>) {
        let encoded = serde_json::to_string(call.original_response).unwrap();
        self.calls.lock().unwrap().push(Recorded {
            input: call.input.to_vec(),
            api_key: call.api_key.to_owned(),
            request_data: call.complete_input_dict.clone(),
            original_response: serde_json::from_str(&encoded).unwrap(),
        });
    }
}

fn claude_body(stop_reason: Option<&str>) -> String {
    let usage = r#""usage":{"input_tokens":3,"output_tokens":4}"#;
    match stop_reason {
        Some(reason) => format!(
            r#"{{"id":"msg_1","model":"claude-3-5-sonnet","content":[{{"type":"text","text":"hi"}}],"stop_reason":"{reason}",{usage}}}"#
        ),
        None => format!(
            r#"{{"id":"msg_1","model":"claude-3-5-sonnet","content":[{{"type":"text","text":"hi"}}],{usage}}}"#
        ),
    }
}

fn claude_reply<'reply>(
    body: &'reply str,
    turns: &'reply [Json],
    request: &'reply Json,
    recorder: &'reply Recorder,
) -> super::ClaudeReply {
    let input = ResponseInput {
        model: "snowflake/claude-3-5-sonnet",
        raw_response: body,
        request_data: request,
        messages: turns,
    };
    transform_response_claude(&input, recorder, &WordCounter).unwrap()
}

#[test]
fn the_openai_reply_prefixes_the_model_and_names_the_hidden_one() {
    let recorder = Recorder::default();
    let turns = vec![parsed(r#"{"role":"user","content":"hi"}"#)];
    let request = parsed(r#"{"model":"mistral-large"}"#);
    let input = ResponseInput {
        model: "snowflake/mistral-large",
        raw_response: OPENAI_REPLY,
        request_data: &request,
        messages: &turns,
    };
    let reply = transform_response_openai(&input, &recorder).unwrap();
    assert_eq!(reply.response.model, "snowflake/mistral-large");
    assert_eq!(
        reply.response.choices[0].message.content.as_deref(),
        Some("hi")
    );
    assert_eq!(reply.response.usage.total_tokens, 3);
    assert_eq!(reply.hidden_model, "snowflake/mistral-large");
}

#[test]
fn an_openai_reply_without_a_model_still_gets_the_prefix() {
    let recorder = Recorder::default();
    let turns = vec![parsed(r#"{"role":"user","content":"hi"}"#)];
    let request = parsed("{}");
    let body = r#"{"id":"chatcmpl-1","object":"chat.completion","created":7,"model":"","choices":[],"usage":{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0}}"#;
    let input = ResponseInput {
        model: "snowflake/mistral-large",
        raw_response: body,
        request_data: &request,
        messages: &turns,
    };
    let reply = transform_response_openai(&input, &recorder).unwrap();
    assert_eq!(reply.response.model, "snowflake/");
    assert!(reply.response.choices.is_empty());
}

#[test]
fn the_openai_reply_logs_the_body_the_request_data_and_an_empty_key() {
    let recorder = Recorder::default();
    let turns = vec![parsed(r#"{"role":"user","content":"hi"}"#)];
    let request = parsed(r#"{"model":"mistral-large","stream":false}"#);
    let input = ResponseInput {
        model: "snowflake/mistral-large",
        raw_response: OPENAI_REPLY,
        request_data: &request,
        messages: &turns,
    };
    transform_response_openai(&input, &recorder).unwrap();
    let calls = recorder.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input, turns);
    assert_eq!(calls[0].api_key, "");
    assert_eq!(calls[0].request_data, request);
    assert_eq!(calls[0].original_response, parsed(OPENAI_REPLY));
}

#[test]
fn the_claude_reply_logs_the_body_the_request_data_and_an_empty_key() {
    let recorder = Recorder::default();
    let turns = vec![parsed(r#"{"role":"user","content":"hi"}"#)];
    let request = parsed(r#"{"model":"claude-3-5-sonnet"}"#);
    let body = claude_body(Some("end_turn"));
    claude_reply(&body, &turns, &request, &recorder);
    let calls = recorder.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input, turns);
    assert_eq!(calls[0].api_key, "");
    assert_eq!(calls[0].request_data, request);
    assert_eq!(calls[0].original_response, parsed(&body));
}

#[test]
fn every_claude_stop_reason_maps_to_an_openai_finish_reason() {
    for (stop_reason, expected) in [
        (Some("end_turn"), "stop"),
        (Some("max_tokens"), "length"),
        (Some("tool_use"), "tool_calls"),
        (Some("stop_sequence"), "stop"),
        (Some("something_else"), "stop"),
        (None, "stop"),
    ] {
        let recorder = Recorder::default();
        let turns = vec![parsed(r#"{"role":"user","content":"hi"}"#)];
        let request = parsed("{}");
        let body = claude_body(stop_reason);
        let reply = claude_reply(&body, &turns, &request, &recorder);
        assert_eq!(reply.choices[0].finish_reason, expected);
    }
}

#[test]
fn a_claude_reply_carries_content_tool_calls_thinking_and_reasoning() {
    let recorder = Recorder::default();
    let turns = vec![parsed(r#"{"role":"user","content":"hi"}"#)];
    let request = parsed("{}");
    let body = r#"{"id":"msg_7","model":"claude-3-5-sonnet","content":[{"type":"thinking","thinking":"plan","signature":"sig"},{"type":"text","text":"call"},{"type":"tool_use","id":"t1","name":"f","input":{"a":1}}],"stop_reason":"tool_use","usage":{"input_tokens":3,"output_tokens":4}}"#;
    let reply = claude_reply(body, &turns, &request, &recorder);
    assert_eq!(reply.id, "msg_7");
    assert_eq!(reply.model, "snowflake/claude-3-5-sonnet");
    assert_eq!(reply.choices.len(), 1);
    assert_eq!(reply.choices[0].index, 0);
    assert_eq!(reply.choices[0].message.role, "assistant");
    assert_eq!(reply.choices[0].message.content.as_deref(), Some("call"));
    let calls = reply.choices[0].message.tool_calls.as_deref().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id.as_deref(), Some("t1"));
    assert_eq!(calls[0].function.name.as_deref(), Some("f"));
    assert_eq!(calls[0].function.arguments.as_deref(), Some(r#"{"a": 1}"#));
    assert_eq!(
        reply.choices[0].message.thinking_blocks.as_deref(),
        Some(
            &[answer(
                r#"{"type":"thinking","thinking":"plan","signature":"sig"}"#
            )][..]
        )
    );
    assert_eq!(
        reply.choices[0].message.reasoning_content.as_deref(),
        Some("plan")
    );
    assert_eq!(reply.choices[0].finish_reason, "tool_calls");
    assert_eq!(reply.usage.prompt_tokens, 3);
    assert_eq!(reply.usage.completion_tokens, 4);
    assert_eq!(reply.usage.total_tokens, 7);
    assert_eq!(reply.usage.completion_tokens_details.reasoning, Some(1));
    assert_eq!(reply.usage.completion_tokens_details.text, Some(3));
    assert_eq!(reply.hidden_model, "snowflake/claude-3-5-sonnet");
}

#[test]
fn a_claude_reply_without_id_model_or_usage_takes_the_defaults() {
    let recorder = Recorder::default();
    let turns = vec![parsed(r#"{"role":"user","content":"hi"}"#)];
    let request = parsed("{}");
    let body = r#"{"content":[{"type":"text","text":"hi"}]}"#;
    let reply = claude_reply(body, &turns, &request, &recorder);
    assert_eq!(reply.id, "");
    assert_eq!(reply.model, "snowflake/snowflake/claude-3-5-sonnet");
    assert_eq!(reply.choices[0].message.content.as_deref(), Some("hi"));
    assert_eq!(reply.usage.prompt_tokens, 0);
    assert_eq!(reply.usage.completion_tokens, 0);
    assert_eq!(reply.usage.total_tokens, 0);
    assert_eq!(reply.usage.completion_tokens_details.reasoning, Some(0));
    assert_eq!(reply.usage.completion_tokens_details.text, Some(0));
}

#[test]
fn a_redacted_thinking_only_reply_leaves_the_reasoning_split_unset() {
    let recorder = Recorder::default();
    let turns = vec![parsed(r#"{"role":"user","content":"hi"}"#)];
    let request = parsed("{}");
    let body = r#"{"id":"msg_1","model":"claude-3-5-sonnet","content":[{"type":"redacted_thinking","data":"opaque"}],"usage":{"input_tokens":1,"output_tokens":4}}"#;
    let reply = claude_reply(body, &turns, &request, &recorder);
    assert_eq!(reply.usage.completion_tokens, 4);
    assert_eq!(reply.usage.completion_tokens_details.reasoning, None);
    assert_eq!(reply.usage.completion_tokens_details.text, None);
}

#[test]
fn a_tool_search_server_block_counts_as_a_tool_search_request() {
    let recorder = Recorder::default();
    let turns = vec![parsed(r#"{"role":"user","content":"hi"}"#)];
    let request = parsed("{}");
    let body = r#"{"id":"msg_1","model":"claude-3-5-sonnet","content":[{"type":"server_tool_use","id":"srv_1","name":"tool_search_tool_regex","input":{}},{"type":"text","text":"hi"}],"usage":{"input_tokens":1,"output_tokens":4}}"#;
    let reply = claude_reply(body, &turns, &request, &recorder);
    let server_tool_use = reply.usage.server_tool_use.unwrap();
    assert_eq!(server_tool_use.tool_search, Some(1));
    assert_eq!(server_tool_use.web_search, None);
}

#[test]
fn an_empty_claude_message_has_no_content_and_no_tool_calls() {
    let recorder = Recorder::default();
    let turns = vec![parsed(r#"{"role":"user","content":"hi"}"#)];
    let request = parsed("{}");
    let body = r#"{"id":"msg_1","model":"claude-3-5-sonnet","content":[],"usage":{"input_tokens":1,"output_tokens":1}}"#;
    let reply = claude_reply(body, &turns, &request, &recorder);
    assert_eq!(reply.choices[0].message.content, None);
    assert_eq!(reply.choices[0].message.tool_calls, None);
    assert_eq!(reply.choices[0].message.thinking_blocks, None);
    assert_eq!(reply.choices[0].message.reasoning_content, None);
}

#[test]
fn an_unreadable_openai_body_is_a_decode_error() {
    let recorder = Recorder::default();
    let turns = vec![parsed(r#"{"role":"user","content":"hi"}"#)];
    let request = parsed("{}");
    let input = ResponseInput {
        model: "snowflake/mistral-large",
        raw_response: "not json",
        request_data: &request,
        messages: &turns,
    };
    let error = transform_response_openai(&input, &recorder).unwrap_err();
    assert!(matches!(error, super::ResponseError::Decode { .. }));
}
