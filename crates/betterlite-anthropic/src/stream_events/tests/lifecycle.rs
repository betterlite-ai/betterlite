use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::fixture::{
    as_json, at, chunk, content, converter_with, new_converter, parse, provider_fields,
    reverse_map, tool_calls,
};
use crate::stream_events::StreamEventError;
use crate::stream_state::json_fragment_buffer::AccumulatedJson as _;

fn web_search_start() {
    let mut converter = new_converter();
    let start = chunk(
        &mut converter,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"server_tool_use","id":"ws1","name":"web_search","input":{"query":"cats"}}}"#,
    );
    assert_eq!(
        provider_fields(&start).unwrap(),
        parse(
            r#"{"web_search_calls":{"ws1":{"id":"ws_ws1","type":"web_search_call","status":"in_progress","action":{"type":"search","query":"cats","queries":["cats"],"sources":[]}}}}"#
        )
    );
    assert_eq!(
        tool_calls(&start).unwrap(),
        parse(
            r#"[{"id":"ws1","type":"function","function":{"name":"web_search","arguments":""},"index":0}]"#
        )
    );
    chunk(
        &mut converter,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"query\": \"dogs\"}"}}"#,
    );
    chunk(&mut converter, r#"{"type":"content_block_stop","index":0}"#);
    assert_eq!(
        as_json(converter.state.server_tool_inputs.get("ws1").unwrap()),
        parse(r#"{"query":"dogs"}"#)
    );
}

fn web_search_result_completes_call() {
    let mut converter = new_converter();
    chunk(
        &mut converter,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"server_tool_use","id":"ws1","name":"web_search","input":{"query":"cats"}}}"#,
    );
    let result = chunk(
        &mut converter,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"web_search_tool_result","tool_use_id":"ws1","content":[{"type":"web_search_result","url":"https://e.x","title":"t"}]}}"#,
    );
    assert_eq!(
        provider_fields(&result).unwrap(),
        parse(
            r#"{"web_search_results":[{"type":"web_search_tool_result","tool_use_id":"ws1","content":[{"type":"web_search_result","url":"https://e.x","title":"t"}]}],"web_search_calls":{"ws1":{"id":"ws_ws1","type":"web_search_call","status":"completed","action":{"type":"search","query":"cats","queries":["cats"],"sources":[{"type":"url","url":"https://e.x"}]}}}}"#
        )
    );
    assert_eq!(converter.state.web_search_results.len(), 1);
}

fn web_search_unmatched_and_fetch() {
    let mut converter = new_converter();
    let unmatched = chunk(
        &mut converter,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"web_search_tool_result","tool_use_id":"other","content":[]}}"#,
    );
    let unmatched_fields = provider_fields(&unmatched).unwrap();
    assert_eq!(
        at(&unmatched_fields, "/web_search_calls").unwrap(),
        &Json::Object(IndexMap::new())
    );
    assert_eq!(
        as_json(converter.state.web_search_results.first().unwrap()),
        parse(r#"{"type":"web_search_tool_result","tool_use_id":"other","content":[]}"#)
    );
    let fetch = chunk(
        &mut converter,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"web_fetch_tool_result","tool_use_id":"wf1","content":{"type":"web_fetch_result","url":"https://y"}}}"#,
    );
    let fetch_fields = provider_fields(&fetch).unwrap();
    assert_eq!(at(&fetch_fields, "/web_search_calls"), None);
    assert_eq!(
        at(&fetch_fields, "/web_search_results/1/type").unwrap(),
        &Json::Text("web_fetch_tool_result".to_owned())
    );
    assert_eq!(converter.state.web_search_results.len(), 2);
}

fn web_search_other_tool_results() {
    let mut converter = new_converter();
    let bash = chunk(
        &mut converter,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"bash_code_execution_tool_result","tool_use_id":"b1","content":{"type":"bash_code_execution_result","stdout":"hi","stderr":""}}}"#,
    );
    let bash_fields = provider_fields(&bash).unwrap();
    assert_eq!(
        at(&bash_fields, "/tool_results/0/type").unwrap(),
        &Json::Text("bash_code_execution_tool_result".to_owned())
    );
    assert_eq!(
        at(&bash_fields, "/code_interpreter_results/0/type").unwrap(),
        &Json::Text("code_interpreter_call".to_owned())
    );
    assert_eq!(
        at(&bash_fields, "/code_interpreter_results/0/status").unwrap(),
        &Json::Text("completed".to_owned())
    );
    assert_eq!(
        at(&bash_fields, "/code_interpreter_results/0/code").unwrap(),
        &Json::Text(String::new())
    );
    let ignored = chunk(
        &mut converter,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_search_tool_result","tool_use_id":"ts1","content":[]}}"#,
    );
    assert_eq!(provider_fields(&ignored), None);
    assert_eq!(converter.state.tool_results.len(), 1);
}

fn web_search_name_variants() {
    let mut other = new_converter();
    let fetch_tool = chunk(
        &mut other,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"server_tool_use","id":"ws2","name":"web_fetch","input":{"url":"https://z"}}}"#,
    );
    assert_eq!(
        at(&provider_fields(&fetch_tool).unwrap(), "/web_search_calls"),
        None
    );
    assert_eq!(
        as_json(other.state.server_tool_inputs.get("ws2").unwrap()),
        parse(r#"{"url":"https://z"}"#)
    );
}

fn web_search_reversed_and_missing_input() {
    let mut reversed = converter_with(None, reverse_map("sanitized_search", "web_search"));
    let mapped = chunk(
        &mut reversed,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"server_tool_use","id":"rs1","name":"sanitized_search","input":{"query":"birds"}}}"#,
    );
    assert_eq!(
        at(
            &provider_fields(&mapped).unwrap(),
            "/web_search_calls/rs1/action/query"
        )
        .unwrap(),
        &Json::Text("birds".to_owned())
    );
    let mut missing = new_converter();
    let no_input = chunk(
        &mut missing,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"server_tool_use","id":"ws3","name":"web_search"}}"#,
    );
    assert_eq!(
        at(
            &provider_fields(&no_input).unwrap(),
            "/web_search_calls/ws3/action/query"
        )
        .unwrap(),
        &Json::Text(String::new())
    );
    assert_eq!(
        as_json(missing.state.server_tool_inputs.get("ws3").unwrap()),
        parse("{}")
    );
}

pub(super) fn web_search_stream_lifecycle() {
    web_search_start();
    web_search_result_completes_call();
    web_search_unmatched_and_fetch();
    web_search_other_tool_results();
    web_search_name_variants();
    web_search_reversed_and_missing_input();
}

fn compaction_starts() {
    let mut converter = new_converter();
    let first = chunk(
        &mut converter,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"compaction","content":"sum"}}"#,
    );
    assert_eq!(
        provider_fields(&first).unwrap(),
        parse(
            r#"{"compaction_blocks":[{"type":"compaction","content":"sum"}],"compaction_start":{"type":"compaction","content":"sum"}}"#
        )
    );
    let second = chunk(
        &mut converter,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"compaction"}}"#,
    );
    assert_eq!(
        at(
            &provider_fields(&second).unwrap(),
            "/compaction_start/content"
        )
        .unwrap(),
        &Json::Text(String::new())
    );
    assert_eq!(converter.state.compaction_blocks.len(), 2);
}

fn container_before_results() {
    let mut converter = new_converter();
    let early = chunk(
        &mut converter,
        r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","container":{"id":"c1"}}}"#,
    );
    assert_eq!(
        provider_fields(&early).unwrap(),
        parse(r#"{"container":{"id":"c1"}}"#)
    );
    assert_eq!(converter.state.container_id, None);
}

fn container_after_results() {
    let mut converter = new_converter();
    chunk(
        &mut converter,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"bash_code_execution_tool_result","tool_use_id":"b1","content":{"type":"bash_code_execution_result","stdout":"hi","stderr":""}}}"#,
    );
    let stored = chunk(
        &mut converter,
        r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","container":{"id":"c9"}}}"#,
    );
    let stored_fields = provider_fields(&stored).unwrap();
    assert_eq!(
        at(&stored_fields, "/code_interpreter_results/0/container_id").unwrap(),
        &Json::Text("c9".to_owned())
    );
    assert_eq!(
        at(&stored_fields, "/container/id").unwrap(),
        &Json::Text("c9".to_owned())
    );
    assert_eq!(converter.state.container_id.as_deref(), Some("c9"));
}

fn container_without_id() {
    let mut converter = new_converter();
    let null_container = chunk(
        &mut converter,
        r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","container":null}}"#,
    );
    assert_eq!(provider_fields(&null_container), None);
    let empty_container = chunk(
        &mut converter,
        r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","container":{}}}"#,
    );
    assert_eq!(provider_fields(&empty_container), None);
    let unidentified = chunk(
        &mut converter,
        r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","container":{"type":"x"}}}"#,
    );
    assert_eq!(
        provider_fields(&unidentified).unwrap(),
        parse(r#"{"container":{"type":"x"}}"#)
    );
    assert_eq!(converter.state.container_id, None);
}

fn container_truthiness() {
    let mut converter = new_converter();
    let empty_id = chunk(
        &mut converter,
        r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","container":{"id":""}}}"#,
    );
    assert_eq!(
        provider_fields(&empty_id).unwrap(),
        parse(r#"{"container":{"id":""}}"#)
    );
    assert_eq!(converter.state.container_id, None);
    let number = chunk(
        &mut converter,
        r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","container":5}}"#,
    );
    assert_eq!(
        provider_fields(&number).unwrap(),
        parse(r#"{"container":5}"#)
    );
    let zero = chunk(
        &mut converter,
        r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","container":0}}"#,
    );
    assert_eq!(provider_fields(&zero), None);
    let empty_text = chunk(
        &mut converter,
        r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","container":""}}"#,
    );
    assert_eq!(provider_fields(&empty_text), None);
    let empty_list = chunk(
        &mut converter,
        r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","container":[]}}"#,
    );
    assert_eq!(provider_fields(&empty_list), None);
}

pub(super) fn compaction_and_container_events() {
    compaction_starts();
    container_before_results();
    container_after_results();
    container_without_id();
    container_truthiness();
}

fn accumulated_complete_and_partial() {
    let mut converter = new_converter();
    let complete = converter
        .handle_accumulated_json_chunk(r#"{"type":"ping"}"#, false)
        .unwrap()
        .unwrap();
    assert_eq!(content(&complete), "");
    assert_eq!(converter.state.json_buffer.snapshot(), "");
    assert!(
        converter
            .handle_accumulated_json_chunk(r#"{"type": "con"#, false)
            .unwrap()
            .is_none()
    );
    assert_eq!(converter.state.json_buffer.snapshot(), r#"{"type": "con"#);
    let completed = converter
        .handle_accumulated_json_chunk(
            r#"tent_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "hi"}}"#,
            false,
        )
        .unwrap()
        .unwrap();
    assert_eq!(content(&completed), "hi");
    assert_eq!(converter.state.json_buffer.snapshot(), "");
}

fn accumulated_multiple_values() {
    let mut converter = new_converter();
    let skipped = converter
        .handle_accumulated_json_chunk(r#"[1,2] {"type":"ping"}"#, false)
        .unwrap()
        .unwrap();
    assert_eq!(content(&skipped), "");
    assert_eq!(converter.state.json_buffer.snapshot(), "");
    let first = converter
        .handle_accumulated_json_chunk(r#"{"type":"ping"}{"type":"message_stop"}"#, false)
        .unwrap()
        .unwrap();
    assert_eq!(content(&first), "");
    assert_eq!(
        converter.state.json_buffer.snapshot(),
        r#"{"type":"message_stop"}"#
    );
    assert!(
        converter
            .handle_accumulated_json_chunk("", false)
            .unwrap()
            .is_some()
    );
    assert!(
        converter
            .handle_accumulated_json_chunk("", false)
            .unwrap()
            .is_none()
    );
}

fn accumulated_drain_and_tail() {
    let mut converter = new_converter();
    assert!(
        converter
            .handle_accumulated_json_chunk(r#"{"type":"ping"}   "#, false)
            .unwrap()
            .is_some()
    );
    assert_eq!(converter.state.json_buffer.snapshot(), "   ");
    assert!(
        converter
            .handle_accumulated_json_chunk(r#"{"ty"#, false)
            .unwrap()
            .is_none()
    );
    assert!(
        converter
            .handle_accumulated_json_chunk("", true)
            .unwrap()
            .is_none()
    );
    assert_eq!(converter.state.json_buffer.snapshot(), r#"   {"ty"#);
    let mut drained = new_converter();
    assert!(
        drained
            .handle_accumulated_json_chunk(r#"{"type":"ping"}x"#, true)
            .unwrap()
            .is_some()
    );
    assert_eq!(drained.state.json_buffer.snapshot(), "x");
    assert!(
        drained
            .handle_accumulated_json_chunk("", true)
            .unwrap()
            .is_none()
    );
    assert_eq!(drained.state.json_buffer.snapshot(), "x");
}

fn accumulated_error_event() {
    let mut converter = new_converter();
    let failure = converter
        .handle_accumulated_json_chunk(r#"{"type":"error","error":{"message":"boom"}}"#, true)
        .unwrap_err();
    assert!(matches!(failure, StreamEventError::Anthropic(_)));
}

fn accumulated_waits_for_closing_byte() {
    let mut converter = new_converter();
    assert!(
        converter
            .handle_accumulated_json_chunk(r#"{"type":"ping"}{"ty"#, false)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        converter.state.json_buffer.snapshot(),
        r#"{"type":"ping"}{"ty"#
    );
    let completed = converter
        .handle_accumulated_json_chunk(r#"pe":"message_stop"}"#, false)
        .unwrap()
        .unwrap();
    assert_eq!(content(&completed), "");
    let second = converter
        .handle_accumulated_json_chunk("", false)
        .unwrap()
        .unwrap();
    assert_eq!(content(&second), "");
    assert_eq!(converter.state.json_buffer.snapshot(), "");
}

pub(super) fn accumulated_json_values() {
    accumulated_complete_and_partial();
    accumulated_multiple_values();
    accumulated_drain_and_tail();
    accumulated_waits_for_closing_byte();
    accumulated_error_event();
}
