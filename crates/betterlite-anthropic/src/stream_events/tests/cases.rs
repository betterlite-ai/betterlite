use indexmap::IndexMap;

use super::fixture::{
    as_json, chunk, content, converter_with, finish_reason, new_converter, parse, provider_fields,
    reverse_map, tool_calls,
};
use crate::stream_events::StreamEventConverter;
use crate::stream_events::StreamEventError;
use crate::stream_events::error::AnthropicError;
use crate::usage::ReasoningTokenCounter;

const RESPONSE_ID: &str = "chatcmpl-01234567-89ab-4def-8123-456789abcdef";

const TOOL_START: &str = r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t1","name":"get_weather_sanitized","input":{"unused":true}}}"#;
const TOOL_DELTA: &str = r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"loc\":"}}"#;
const TOOL_DELTA_TAIL: &str = r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"\"SF\"}"}}"#;
const TOOL_STOP: &str = r#"{"type":"content_block_stop","index":0}"#;

fn assert_empty_chunk<Counter>(converter: &mut StreamEventConverter<Counter>, source: &str)
where
    Counter: ReasoningTokenCounter,
{
    let empty = chunk(converter, source);
    assert_eq!(content(&empty), "");
    assert_eq!(finish_reason(&empty), None);
    assert_eq!(tool_calls(&empty), None);
    assert_eq!(provider_fields(&empty), None);
    assert_eq!(empty.usage, None);
    assert_eq!(empty.model, None);
    assert_eq!(empty.id, RESPONSE_ID);
}

fn assert_native_error<Counter>(converter: &mut StreamEventConverter<Counter>, source: &str)
where
    Counter: ReasoningTokenCounter,
{
    let failure = converter.chunk_parser(&parse(source)).unwrap_err();
    let StreamEventError::Anthropic(AnthropicError { status_code, .. }) = failure else {
        panic!("a native error event is not a shape failure: {failure}");
    };
    assert_eq!(status_code, 500);
}

fn message_start_copies_model() {
    let mut converter = new_converter();
    let start = chunk(
        &mut converter,
        r#"{"type":"message_start","message":{"id":"msg_1","role":"assistant","model":"claude-x","usage":{"input_tokens":10,"output_tokens":1}}}"#,
    );
    assert_eq!(start.id, RESPONSE_ID);
    assert_eq!(start.model.as_deref(), Some("claude-x"));
    assert_eq!(start.choices.len(), 1);
    assert_eq!(start.choices[0].index, 0);
    assert_eq!(content(&start), "");
    assert_eq!(finish_reason(&start), None);
    assert_eq!(tool_calls(&start), None);
    assert_eq!(provider_fields(&start), None);
    let usage = start.usage.as_ref().unwrap();
    assert_eq!(usage.prompt_tokens, 10);
    assert_eq!(usage.completion_tokens, 1);
    let ping = chunk(&mut converter, r#"{"type":"ping"}"#);
    assert_eq!(ping.id, RESPONSE_ID);
    assert_eq!(ping.model.as_deref(), Some("claude-x"));
}

fn message_start_model_presence() {
    let mut converter = new_converter();
    chunk(
        &mut converter,
        r#"{"type":"message_start","message":{"model":"claude-x"}}"#,
    );
    let missing = chunk(
        &mut converter,
        r#"{"type":"message_start","message":{"id":"msg_2"}}"#,
    );
    assert_eq!(missing.model.as_deref(), Some("claude-x"));
    assert_eq!(missing.usage, None);
    let cleared = chunk(
        &mut converter,
        r#"{"type":"message_start","message":{"model":null}}"#,
    );
    assert_eq!(cleared.model, None);
}

fn message_start_usage_merges() {
    let mut converter = new_converter();
    chunk(
        &mut converter,
        r#"{"type":"message_start","message":{"usage":{"input_tokens":10,"output_tokens":1}}}"#,
    );
    let delta = chunk(
        &mut converter,
        r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":5}}"#,
    );
    let merged = delta.usage.as_ref().unwrap();
    assert_eq!(merged.prompt_tokens, 10);
    assert_eq!(merged.completion_tokens, 5);
    assert_eq!(merged.total_tokens, 15);
}

pub(super) fn message_start_role() {
    message_start_copies_model();
    message_start_model_presence();
    message_start_usage_merges();
}

pub(super) fn text_delta() {
    let mut converter = new_converter();
    let start = chunk(
        &mut converter,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":"Hello"}}"#,
    );
    assert_eq!(content(&start), "Hello");
    assert_eq!(tool_calls(&start), None);
    let delta = chunk(
        &mut converter,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":" world"}}"#,
    );
    assert_eq!(content(&delta), " world");
    assert_eq!(delta.choices[0].delta.provider_specific_fields, None);
    let empty = chunk(
        &mut converter,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":""}}"#,
    );
    assert_eq!(content(&empty), "");
    let stop = chunk(
        &mut converter,
        r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#,
    );
    assert_eq!(finish_reason(&stop), Some("stop"));
    assert_eq!(content(&stop), "");
    let length = chunk(
        &mut converter,
        r#"{"type":"message_delta","delta":{"stop_reason":"max_tokens"}}"#,
    );
    assert_eq!(finish_reason(&length), Some("length"));
    let absent = chunk(&mut converter, r#"{"type":"message_delta","delta":{}}"#);
    assert_eq!(finish_reason(&absent), Some("stop"));
}

fn tool_start_and_deltas() {
    let mut converter = converter_with(None, reverse_map("get_weather_sanitized", "get_weather"));
    let start = chunk(&mut converter, TOOL_START);
    assert_eq!(
        tool_calls(&start).unwrap(),
        parse(
            r#"[{"id":"t1","type":"function","function":{"name":"get_weather","arguments":""},"index":0}]"#
        )
    );
    let delta = chunk(&mut converter, TOOL_DELTA);
    assert_eq!(
        tool_calls(&delta).unwrap(),
        parse(
            r#"[{"id":null,"type":"function","function":{"name":null,"arguments":"{\"loc\":"},"index":0}]"#
        )
    );
    let tail = chunk(&mut converter, TOOL_DELTA_TAIL);
    assert_eq!(
        tool_calls(&tail).unwrap(),
        parse(
            r#"[{"id":null,"type":"function","function":{"name":null,"arguments":"\"SF\"}"},"index":0}]"#
        )
    );
    let stop = chunk(&mut converter, TOOL_STOP);
    assert_eq!(tool_calls(&stop), None);
    assert_eq!(converter.state.current_content_block_type, None);
    assert_eq!(converter.state.current_server_tool_id, None);
}

fn tool_stop_arguments() {
    let mut converter = new_converter();
    chunk(
        &mut converter,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t2","name":"other","input":{}}}"#,
    );
    let delta_free = chunk(&mut converter, TOOL_STOP);
    assert_eq!(tool_calls(&delta_free), None);
    let third = chunk(
        &mut converter,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t3","name":"third","input":{}}}"#,
    );
    assert_eq!(
        third.choices[0].delta.tool_calls.as_ref().unwrap()[0].index,
        1
    );
    let empty_delta = chunk(
        &mut converter,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":""}}"#,
    );
    assert_eq!(
        tool_calls(&empty_delta).unwrap(),
        parse(
            r#"[{"id":null,"type":"function","function":{"name":null,"arguments":""},"index":1}]"#
        )
    );
    let stop = chunk(&mut converter, TOOL_STOP);
    assert_eq!(
        tool_calls(&stop).unwrap(),
        parse(
            r#"[{"id":null,"type":"function","function":{"name":null,"arguments":"{}"},"index":1}]"#
        )
    );
    assert!(!converter.state.is_response_format_tool);
}

fn server_tool_stop_inputs() {
    let mut converter = new_converter();
    chunk(
        &mut converter,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"server_tool_use","id":"s1","name":"bash_code_execution","input":{"command":"ls"}}}"#,
    );
    chunk(
        &mut converter,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"command\": "}}"#,
    );
    chunk(&mut converter, TOOL_STOP);
    assert_eq!(
        as_json(converter.state.server_tool_inputs.get("s1").unwrap()),
        parse(r#"{"command":"ls"}"#)
    );
    assert_eq!(converter.state.current_server_tool_id, None);
    chunk(
        &mut converter,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"server_tool_use","id":"s2","name":"bash_code_execution","input":{"command":"pwd"}}}"#,
    );
    chunk(
        &mut converter,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"command\": "}}"#,
    );
    chunk(
        &mut converter,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"\"ls -l\"}"}}"#,
    );
    let stop = chunk(&mut converter, TOOL_STOP);
    assert_eq!(tool_calls(&stop), None);
    assert_eq!(
        as_json(converter.state.server_tool_inputs.get("s2").unwrap()),
        parse(r#"{"command":"ls -l"}"#)
    );
    assert_eq!(converter.state.current_server_tool_id, None);
}

fn tool_stop_pending_server_id() {
    let mut converter = new_converter();
    chunk(
        &mut converter,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"server_tool_use","id":"s9","name":"bash_code_execution","input":{"command":"kept"}}}"#,
    );
    chunk(
        &mut converter,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t9","name":"plain","input":{}}}"#,
    );
    chunk(
        &mut converter,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"citations_delta","partial_json":"{\"x\": 1}"}}"#,
    );
    let stop = chunk(&mut converter, TOOL_STOP);
    assert_eq!(
        tool_calls(&stop).unwrap(),
        parse(
            r#"[{"id":null,"type":"function","function":{"name":null,"arguments":"{}"},"index":1}]"#
        )
    );
    assert_eq!(
        as_json(converter.state.server_tool_inputs.get("s9").unwrap()),
        parse(r#"{"command":"kept"}"#)
    );
    assert_eq!(
        converter.state.current_server_tool_id.as_deref(),
        Some("s9")
    );
}

fn server_tool_stop_assembles_only_input_json() {
    let mut converter = new_converter();
    chunk(
        &mut converter,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"server_tool_use","id":"s10","name":"bash_code_execution","input":{"command":"initial"}}}"#,
    );
    chunk(
        &mut converter,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"a\": 1}"}}"#,
    );
    chunk(
        &mut converter,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"citations_delta","partial_json":"{\"b\": 2}"}}"#,
    );
    chunk(&mut converter, TOOL_STOP);
    assert_eq!(
        as_json(converter.state.server_tool_inputs.get("s10").unwrap()),
        parse(r#"{"a":1}"#)
    );
}

fn tool_stop_ignores_a_cited_text_block() {
    let mut converter = new_converter();
    chunk(
        &mut converter,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":"cited"}}"#,
    );
    chunk(
        &mut converter,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"citations_delta","citation":{"type":"web_search_result_location","url":"https://q"}}}"#,
    );
    let stop = chunk(&mut converter, TOOL_STOP);
    assert_eq!(tool_calls(&stop), None);
    assert_eq!(content(&stop), "");
}

fn server_tool_stop_without_deltas() {
    let mut converter = new_converter();
    chunk(
        &mut converter,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"server_tool_use","id":"s11","name":"bash_code_execution","input":{"command":"kept"}}}"#,
    );
    let stop = chunk(&mut converter, TOOL_STOP);
    assert_eq!(tool_calls(&stop), None);
    assert_eq!(
        as_json(converter.state.server_tool_inputs.get("s11").unwrap()),
        parse(r#"{"command":"kept"}"#)
    );
    assert_eq!(converter.state.current_server_tool_id, None);
}

pub(super) fn tool_use_and_argument_delta() {
    tool_start_and_deltas();
    tool_stop_arguments();
    server_tool_stop_inputs();
    server_tool_stop_assembles_only_input_json();
    server_tool_stop_without_deltas();
    tool_stop_pending_server_id();
    tool_stop_ignores_a_cited_text_block();
}

fn thinking_delta_events() {
    let mut converter = new_converter();
    let start = chunk(
        &mut converter,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#,
    );
    assert_eq!(start.choices[0].delta.thinking_blocks, None);
    assert_eq!(start.choices[0].delta.reasoning_content, None);
    assert_eq!(provider_fields(&start), None);
    let thought = chunk(
        &mut converter,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hmm"}}"#,
    );
    assert_eq!(
        provider_fields(&thought).unwrap(),
        parse(r#"{"thinking_blocks":[{"type":"thinking","thinking":"hmm"}]}"#)
    );
    assert_eq!(
        as_json(thought.choices[0].delta.thinking_blocks.as_ref().unwrap()),
        parse(r#"[{"type":"thinking","thinking":"hmm"}]"#)
    );
    assert_eq!(
        thought.choices[0].delta.reasoning_content.as_deref(),
        Some("hmm")
    );
    let more = chunk(
        &mut converter,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":" more"}}"#,
    );
    assert_eq!(
        more.choices[0].delta.reasoning_content.as_deref(),
        Some(" more")
    );
}

fn signature_and_redacted_events() {
    let mut converter = new_converter();
    chunk(
        &mut converter,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hmm"}}"#,
    );
    let signature = chunk(
        &mut converter,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig-abc"}}"#,
    );
    assert_eq!(
        as_json(signature.choices[0].delta.thinking_blocks.as_ref().unwrap()),
        parse(r#"[{"type":"thinking","thinking":"hmm","signature":"sig-abc"}]"#)
    );
    assert_eq!(
        signature.choices[0].delta.reasoning_content.as_deref(),
        Some("")
    );
    let empty_signature = chunk(
        &mut converter,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":""}}"#,
    );
    assert_eq!(empty_signature.choices[0].delta.thinking_blocks, None);
    let redacted = chunk(
        &mut converter,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"redacted_thinking","data":"opaque"}}"#,
    );
    assert_eq!(
        as_json(redacted.choices[0].delta.thinking_blocks.as_ref().unwrap()),
        parse(r#"[{"type":"redacted_thinking","data":"opaque"}]"#)
    );
    assert_eq!(
        as_json(
            redacted.choices[0]
                .delta
                .provider_specific_fields
                .as_ref()
                .unwrap()
        ),
        parse(r#"{"thinking_blocks":[{"type":"redacted_thinking","data":"opaque"}]}"#)
    );
}

pub(super) fn thinking_and_signature_events() {
    thinking_delta_events();
    signature_and_redacted_events();
}

pub(super) fn citation_event() {
    let mut converter = new_converter();
    let citation = chunk(
        &mut converter,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"citations_delta","citation":{"type":"web_search_result_location","url":"https://x","title":"t"}}}"#,
    );
    assert_eq!(
        provider_fields(&citation).unwrap(),
        parse(
            r#"{"citation":{"type":"web_search_result_location","url":"https://x","title":"t"}}"#
        )
    );
    assert_eq!(citation.choices[0].delta.thinking_blocks, None);
    assert_eq!(content(&citation), "");
}

fn json_mode_start_and_fragments() {
    let mut converter = converter_with(Some(true), IndexMap::new());
    let start = chunk(
        &mut converter,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"j1","name":"json_tool_call","input":{}}}"#,
    );
    assert_eq!(content(&start), "");
    assert_eq!(tool_calls(&start), None);
    assert!(converter.state.is_response_format_tool);
    assert!(converter.state.converted_response_format_tool);
    let empty_fragment = chunk(
        &mut converter,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":""}}"#,
    );
    assert_eq!(content(&empty_fragment), "");
    assert_eq!(tool_calls(&empty_fragment), None);
    let values = chunk(
        &mut converter,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"values\": {\"a\": 1}}"}}"#,
    );
    assert_eq!(content(&values), r#"{"a": 1}"#);
    assert_eq!(tool_calls(&values), None);
    let plain = chunk(
        &mut converter,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"b\": 2}"}}"#,
    );
    assert_eq!(content(&plain), r#"{"b": 2}"#);
}

fn json_mode_stop_and_real_tool() {
    let mut converter = converter_with(Some(true), IndexMap::new());
    chunk(
        &mut converter,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"j1","name":"json_tool_call","input":{}}}"#,
    );
    let malformed = chunk(
        &mut converter,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"not json"}}"#,
    );
    assert_eq!(content(&malformed), "not json");
    let stop = chunk(&mut converter, TOOL_STOP);
    assert!(!converter.state.is_response_format_tool);
    assert!(converter.state.converted_response_format_tool);
    assert_eq!(content(&stop), "");
    let real = chunk(
        &mut converter,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"r1","name":"real_tool","input":{}}}"#,
    );
    assert!(!converter.state.is_response_format_tool);
    assert_eq!(
        tool_calls(&real).unwrap(),
        parse(
            r#"[{"id":"r1","type":"function","function":{"name":"real_tool","arguments":""},"index":1}]"#
        )
    );
    let final_delta = chunk(
        &mut converter,
        r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
    );
    assert_eq!(finish_reason(&final_delta), Some("stop"));
}

fn json_mode_name_recognition() {
    let mut unmapped = converter_with(Some(true), IndexMap::new());
    let extra = chunk(
        &mut unmapped,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"x1","name":"json_tool_call_extra","input":{}}}"#,
    );
    assert!(!unmapped.state.is_response_format_tool);
    assert!(tool_calls(&extra).is_some());
    let mut reversed = converter_with(Some(true), reverse_map("sanitized", "json_tool_call"));
    let mapped = chunk(
        &mut reversed,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"x2","name":"sanitized","input":{}}}"#,
    );
    assert!(reversed.state.is_response_format_tool);
    assert_eq!(tool_calls(&mapped), None);
    let mut disabled = new_converter();
    let untouched = chunk(
        &mut disabled,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"x3","name":"json_tool_call","input":{}}}"#,
    );
    assert!(tool_calls(&untouched).is_some());
}

fn json_mode_recognition_resets_by_name() {
    let mut converter = converter_with(Some(true), IndexMap::new());
    chunk(
        &mut converter,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"j2","name":"json_tool_call","input":{}}}"#,
    );
    assert!(converter.state.is_response_format_tool);
    let real = chunk(
        &mut converter,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"r2","name":"real_tool","input":{}}}"#,
    );
    assert!(!converter.state.is_response_format_tool);
    assert_eq!(
        tool_calls(&real).unwrap(),
        parse(
            r#"[{"id":"r2","type":"function","function":{"name":"real_tool","arguments":""},"index":1}]"#
        )
    );
}

fn json_mode_empty_delta_stop_fragment() {
    let mut converter = converter_with(Some(true), IndexMap::new());
    chunk(
        &mut converter,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"j3","name":"json_tool_call","input":{}}}"#,
    );
    chunk(
        &mut converter,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":""}}"#,
    );
    let stop = chunk(&mut converter, TOOL_STOP);
    assert_eq!(content(&stop), "");
    assert_eq!(
        tool_calls(&stop).unwrap(),
        parse(
            r#"[{"id":null,"type":"function","function":{"name":null,"arguments":"{}"},"index":0}]"#
        )
    );
}

pub(super) fn json_mode_chunks() {
    json_mode_start_and_fragments();
    json_mode_stop_and_real_tool();
    json_mode_name_recognition();
    json_mode_recognition_resets_by_name();
    json_mode_empty_delta_stop_fragment();
}

pub(super) fn ping_no_content() {
    let mut converter = new_converter();
    assert_empty_chunk(&mut converter, r#"{"type":"ping"}"#);
    assert_empty_chunk(&mut converter, r#"{"type":"tool_result"}"#);
    assert_empty_chunk(&mut converter, r#"{"type":"message_stop"}"#);
    assert_empty_chunk(&mut converter, r#"{"type":"whatever"}"#);
    assert_empty_chunk(&mut converter, "{}");
    assert_empty_chunk(&mut converter, r#"{"type":"null"}"#);
    assert_empty_chunk(&mut converter, r#"{"type":0}"#);
    assert_empty_chunk(&mut converter, r#"{"type":""}"#);
    assert_eq!(converter.state.response_id, RESPONSE_ID);
}

pub(super) fn error_event() {
    let mut converter = new_converter();
    assert_native_error(
        &mut converter,
        r#"{"type":"error","error":{"type":"api_error","message":"boom"}}"#,
    );
    assert_native_error(
        &mut converter,
        r#"{"type":"error","error":{"type":"rate_limit_error","message":"slow"}}"#,
    );
    assert_native_error(
        &mut converter,
        r#"{"type":"error","error":{"type":"authentication_error","message":"bad key"}}"#,
    );
    assert_native_error(
        &mut converter,
        r#"{"type":"error","error":{"type":"something_new","message":"odd"}}"#,
    );
    assert_native_error(
        &mut converter,
        r#"{"type":"error","error":{"message":"no type"}}"#,
    );
    assert_native_error(&mut converter, r#"{"type":"error","error":{}}"#);
    assert_native_error(&mut converter, r#"{"type":"error"}"#);
    assert_native_error(&mut converter, r#"{"type":"error","error":null}"#);
    assert_native_error(&mut converter, r#"{"type":"error","error":false}"#);
    assert_native_error(
        &mut converter,
        r#"{"type":"error","error":{"message":null}}"#,
    );
    assert_native_error(&mut converter, r#"{"type":"error","error":{"message":""}}"#);
    native_error_messages();
}

fn native_error_messages() {
    let mut converter = new_converter();
    let detailed = converter
        .chunk_parser(&parse(r#"{"type":"error","error":{"message":"boom"}}"#))
        .unwrap_err();
    assert_eq!(detailed.to_string(), "boom");
    let fallback = converter
        .chunk_parser(&parse(r#"{"type":"error","error":{}}"#))
        .unwrap_err()
        .to_string();
    assert!(fallback.contains("type"));
    assert!(fallback.contains("error"));
    let empty_message = converter
        .chunk_parser(&parse(r#"{"type":"error","error":{"message":""}}"#))
        .unwrap_err()
        .to_string();
    assert!(empty_message.contains("type"));
}

fn content_start_resets_blocks() {
    let mut converter = new_converter();
    chunk(&mut converter, TOOL_START);
    chunk(
        &mut converter,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":""}}"#,
    );
    assert_eq!(converter.state.content_blocks.len(), 1);
    let next = chunk(
        &mut converter,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t2","name":"second","input":{}}}"#,
    );
    assert!(converter.state.content_blocks.is_empty());
    assert_eq!(
        next.choices[0].delta.tool_calls.as_ref().unwrap()[0].index,
        1
    );
    let stop = chunk(&mut converter, TOOL_STOP);
    assert_eq!(tool_calls(&stop), None);
}

fn content_start_tracks_block_type() {
    let mut converter = new_converter();
    let text = chunk(
        &mut converter,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":"next"}}"#,
    );
    assert_eq!(content(&text), "next");
    assert_eq!(
        converter.state.current_content_block_type.as_deref(),
        Some("text")
    );
    let suppressed = chunk(
        &mut converter,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"zzz"}}"#,
    );
    assert_eq!(tool_calls(&suppressed), None);
    assert_eq!(suppressed.choices[0].delta.provider_specific_fields, None);
}

fn content_start_tool_index_and_caller() {
    let mut converter = new_converter();
    let caller = chunk(
        &mut converter,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"c1","name":"called","input":{},"caller":{"type":"direct"}}}"#,
    );
    assert_eq!(
        tool_calls(&caller).unwrap(),
        parse(
            r#"[{"id":"c1","type":"function","function":{"name":"called","arguments":""},"index":0,"caller":{"type":"direct"}}]"#
        )
    );
    let empty_caller = chunk(
        &mut converter,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"c2","name":"called","input":{},"caller":{}}}"#,
    );
    assert_eq!(
        empty_caller.choices[0].delta.tool_calls.as_ref().unwrap()[0].caller,
        None
    );
    let null_caller = chunk(
        &mut converter,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"c3","name":"called","input":{},"caller":null}}"#,
    );
    assert_eq!(
        null_caller.choices[0].delta.tool_calls.as_ref().unwrap()[0].caller,
        None
    );
}

fn content_start_required_fields() {
    let mut converter = new_converter();
    let missing_text = converter
        .chunk_parser(&parse(
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#,
        ))
        .unwrap_err();
    assert!(missing_text.to_string().contains("content_block.text"));
    let missing_id = converter
        .chunk_parser(&parse(
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","name":"f","input":{}}}"#,
        ))
        .unwrap_err();
    assert!(missing_id.to_string().contains("content_block.id"));
    // LiteLLM bumps the tool index before it reads the required block fields
    assert_eq!(converter.state.tool_index, 1);
}

pub(super) fn content_start_reset() {
    content_start_resets_blocks();
    content_start_tracks_block_type();
    content_start_tool_index_and_caller();
    content_start_required_fields();
}
