use std::convert::Infallible;
use std::sync::{Mutex, PoisonError};

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use serde::Serialize;
use uuid::Uuid;

use crate::stream_state::code::build_code_interpreter_results;
use crate::stream_state::error::StreamStateError;
use crate::stream_state::events::{
    ContentBlockStartShape, DeltaProjection, content_block_delta_helper, get_content_block_start,
    redacted_thinking_content,
};
use crate::stream_state::json_fragment_buffer::AccumulatedJson as _;
use crate::stream_state::message::{handle_message_delta, handle_usage};
use crate::stream_state::state::{
    ChunkType, IdSource, StreamOptions, StreamState, UuidV4, check_empty_tool_call_args,
};
use crate::stream_state::web::{complete_web_search_call, web_search_call_snapshot};
use crate::stream_state::wire::{ContentBlockDelta, JsonFields};
use crate::usage::ReasoningTokenCounter;

const RESPONSE_UUID: u128 = 0x0123_4567_89ab_4def_8123_4567_89ab_cdef;
const RESPONSE_ID: &str = "chatcmpl-01234567-89ab-4def-8123-456789abcdef";

const TEXT_DELTA: &str =
    r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello"}}"#;
const EMPTY_INPUT_JSON: &str =
    r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta"}}"#;

struct FixedIds(Uuid);

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        self.0
    }
}

struct RecordingCounter {
    count: usize,
    texts: Mutex<Vec<String>>,
}

impl RecordingCounter {
    fn new(count: usize) -> Self {
        Self {
            count,
            texts: Mutex::new(Vec::new()),
        }
    }

    fn texts(&self) -> Vec<String> {
        self.texts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ReasoningTokenCounter for RecordingCounter {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, text: &str) -> Result<usize, Infallible> {
        self.texts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(text.to_owned());
        Ok(self.count)
    }
}

fn fixed_ids() -> FixedIds {
    FixedIds(Uuid::from_u128(RESPONSE_UUID))
}

fn state_with(json_mode: Option<bool>, speed: Option<&str>) -> StreamState {
    StreamState::new(
        StreamOptions {
            sync_stream: true,
            json_mode,
            speed: speed.map(str::to_owned),
            tool_name_reverse_map: IndexMap::from([("old".to_owned(), "new".to_owned())]),
        },
        &fixed_ids(),
    )
}

fn default_state() -> StreamState {
    state_with(Some(false), None)
}

fn chunk(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn as_json<Value>(value: &Value) -> Json
where
    Value: Serialize,
{
    serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
}

fn expected(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn block(source: &str) -> ContentBlockDelta {
    serde_json::from_str(source).unwrap()
}

fn delta(state: &mut StreamState, source: &str) -> DeltaProjection {
    content_block_delta_helper(state, &chunk(source)).unwrap()
}

fn assert_delta(state: &mut StreamState, source: &str, expected_source: &str) {
    assert_eq!(as_json(&delta(state, source)), expected(expected_source));
}

fn assert_initial_identity(state: &StreamState) {
    assert_eq!(state.response_id, RESPONSE_ID);
    assert_eq!(state.served_model, None);
    assert_eq!(state.tool_index, -1);
    assert_eq!(state.json_mode, Some(true));
    assert_eq!(state.speed.as_deref(), Some("fast"));
    assert_eq!(
        state.tool_name_reverse_map,
        IndexMap::from([("old".to_owned(), "new".to_owned())])
    );
    assert_eq!(state.chunk_type, ChunkType::ValidJson);
    assert_eq!(state.current_content_block_type, None);
    assert_eq!(state.current_server_tool_id, None);
    assert_eq!(state.container_id, None);
    assert!(!state.is_response_format_tool);
    assert!(!state.converted_response_format_tool);
}

fn assert_initial_buffers(state: &StreamState) {
    assert!(state.content_blocks.is_empty());
    assert!(state.cumulative_usage.is_empty());
    assert!(state.web_search_results.is_empty());
    assert!(state.web_search_calls.is_empty());
    assert!(state.compaction_blocks.is_empty());
    assert!(state.reasoning_content_chunks.is_empty());
    assert!(state.server_tool_inputs.is_empty());
    assert!(state.tool_results.is_empty());
    assert_eq!(state.json_buffer.snapshot(), "");
}

pub(super) fn initial_state() {
    let state = state_with(Some(true), Some("fast"));
    assert_initial_identity(&state);
    assert_initial_buffers(&state);

    let mut default = default_state();
    assert_eq!(default.json_mode, Some(false));
    assert_eq!(default.speed, None);
    assert!(default.tool_name_reverse_map.is_empty());
    default.json_buffer.set("abc".to_owned());
    assert_eq!(default.json_buffer.snapshot(), "abc");
}

fn empty_args_first_blocks(state: &mut StreamState) {
    assert!(!check_empty_tool_call_args(state));
    state.content_blocks = vec![block(TEXT_DELTA)];
    assert!(!check_empty_tool_call_args(state));
    state.content_blocks = vec![
        block(TEXT_DELTA),
        block(
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"a\":1}"}}"#,
        ),
    ];
    assert!(!check_empty_tool_call_args(state));
    state.content_blocks = vec![block(
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":""}}"#,
    )];
    assert!(check_empty_tool_call_args(state));
    state.content_blocks = vec![block(EMPTY_INPUT_JSON)];
    assert!(check_empty_tool_call_args(state));
}

fn empty_args_input_json(state: &mut StreamState) {
    state.content_blocks = vec![block(
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"a\":1}"}}"#,
    )];
    assert!(!check_empty_tool_call_args(state));
    state.content_blocks = vec![block(
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"   "}}"#,
    )];
    assert!(!check_empty_tool_call_args(state));
    state.content_blocks = vec![
        block(
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","partial_json":"x"}}"#,
        ),
        block(EMPTY_INPUT_JSON),
    ];
    assert!(!check_empty_tool_call_args(state));
}

pub(super) fn empty_tool_arguments() {
    let mut state = default_state();
    empty_args_first_blocks(&mut state);
    empty_args_input_json(&mut state);
}

fn partial_input_block_types(state: &mut StreamState) {
    state.current_content_block_type = Some("tool_use".to_owned());
    state.tool_index = 2;
    assert_delta(
        state,
        r#"{"type":"content_block_delta","index":3,"delta":{"type":"input_json_delta","partial_json":"{\"a\": 1"}}"#,
        r#"{"text":"","tool_use":{"id":null,"type":"function","function":{"name":null,"arguments":"{\"a\": 1"},"index":2},"thinking_blocks":[],"provider_specific_fields":{},"reasoning_content":null}"#,
    );
    assert_eq!(state.content_blocks.len(), 1);
    assert_eq!(state.content_blocks[0].index, 3);

    state.current_content_block_type = Some("server_tool_use".to_owned());
    state.tool_index = 0;
    assert_delta(
        state,
        r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{}"}}"#,
        r#"{"text":"","tool_use":{"id":null,"type":"function","function":{"name":null,"arguments":"{}"},"index":0},"thinking_blocks":[],"provider_specific_fields":{},"reasoning_content":null}"#,
    );

    state.current_content_block_type = Some("web_search_tool_result".to_owned());
    state.tool_index = 1;
    let projection = delta(
        state,
        r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{}"}}"#,
    );
    assert_eq!(projection.tool_use, None);
}

fn partial_input_text_wins(state: &mut StreamState) {
    let before = state.content_blocks.len();
    let projection = delta(
        state,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello","partial_json":"ignored"}}"#,
    );
    assert_eq!(projection.text, "hello");
    assert_eq!(projection.tool_use, None);
    assert!(projection.provider_specific_fields.is_empty());
    assert_eq!(state.content_blocks.len(), before);
}

fn partial_input_provider_fields(state: &mut StreamState) {
    state.current_content_block_type = Some("text".to_owned());
    assert_delta(
        state,
        r#"{"type":"content_block_delta","index":2,"delta":{"type":"citations_delta","citation":{"cited_text":"x"}}}"#,
        r#"{"text":"","tool_use":null,"thinking_blocks":[],"provider_specific_fields":{"citation":{"cited_text":"x"}},"reasoning_content":null}"#,
    );
    let precedence = delta(
        state,
        r#"{"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{}","citation":{"cited_text":"y"}}}"#,
    );
    assert!(precedence.provider_specific_fields.is_empty());
    assert_eq!(precedence.tool_use, None);
    assert_delta(
        state,
        r#"{"type":"content_block_delta","index":2,"delta":{"type":"compaction_delta","content":{"summary":"s"}}}"#,
        r#"{"text":"","tool_use":null,"thinking_blocks":[],"provider_specific_fields":{"compaction_delta":{"type":"compaction_delta","content":{"summary":"s"}}},"reasoning_content":null}"#,
    );
    let non_compaction = delta(
        state,
        r#"{"type":"content_block_delta","index":2,"delta":{"type":"text_delta","content":{"ignored":true}}}"#,
    );
    assert!(non_compaction.provider_specific_fields.is_empty());
    assert_eq!(state.content_blocks.len(), 7);
    assert_eq!(state.content_blocks[3].delta.kind, "citations_delta");
    assert_eq!(state.content_blocks[6].delta.kind, "text_delta");
}

pub(super) fn partial_input_json() {
    let mut state = default_state();
    partial_input_block_types(&mut state);
    partial_input_text_wins(&mut state);
    partial_input_provider_fields(&mut state);
}

fn thinking_blocks(state: &mut StreamState) {
    assert_delta(
        state,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"alpha"}}"#,
        r#"{"text":"","tool_use":null,"thinking_blocks":[{"type":"thinking","thinking":"alpha"}],"provider_specific_fields":{"thinking_blocks":[{"type":"thinking","thinking":"alpha"}]},"reasoning_content":"alpha"}"#,
    );
    assert_delta(
        state,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":""}}"#,
        r#"{"text":"","tool_use":null,"thinking_blocks":[],"provider_specific_fields":{},"reasoning_content":null}"#,
    );
    assert_delta(
        state,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig"}}"#,
        r#"{"text":"","tool_use":null,"thinking_blocks":[{"type":"thinking","signature":"sig","thinking":"alpha"}],"provider_specific_fields":{"thinking_blocks":[{"type":"thinking","signature":"sig","thinking":"alpha"}]},"reasoning_content":""}"#,
    );
    assert_delta(
        state,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"beta","signature":"sig2"}}"#,
        r#"{"text":"","tool_use":null,"thinking_blocks":[{"type":"thinking","signature":"sig2","thinking":"alphabeta"}],"provider_specific_fields":{"thinking_blocks":[{"type":"thinking","signature":"sig2","thinking":"alphabeta"}]},"reasoning_content":"beta"}"#,
    );
    state.content_blocks = vec![block(
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"gamma"}}"#,
    )];
    assert_delta(
        state,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig3"}}"#,
        r#"{"text":"","tool_use":null,"thinking_blocks":[{"type":"thinking","signature":"sig3","thinking":"gamma"}],"provider_specific_fields":{"thinking_blocks":[{"type":"thinking","signature":"sig3","thinking":"gamma"}]},"reasoning_content":""}"#,
    );
}

fn thinking_non_string(state: &mut StreamState) {
    let thinking = delta(
        state,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":5}}"#,
    );
    assert!(thinking.thinking_blocks.is_empty());
    assert_eq!(thinking.reasoning_content, None);
    let signature = delta(
        state,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":7}}"#,
    );
    assert!(signature.thinking_blocks.is_empty());
    assert_eq!(signature.reasoning_content, None);
}

pub(super) fn thinking_signature() {
    let mut state = default_state();
    thinking_blocks(&mut state);
    thinking_non_string(&mut state);
    assert_eq!(state.reasoning_content_chunks, ["alpha", "beta"]);
    assert_eq!(state.content_blocks.len(), 4);
}

fn start_tool_use() {
    let start = get_content_block_start(&chunk(
        r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"t1","name":"f","input":{"x":1},"caller":{"caller_id":"c1"}}}"#,
    ))
    .unwrap();
    assert_eq!(start.shape, ContentBlockStartShape::ToolUse);
    assert_eq!(
        as_json(&start.content_block),
        expected(
            r#"{"type":"tool_use","text":null,"id":"t1","name":"f","input":{"x":1},"data":null,"content":null,"caller":{"caller_id":"c1"}}"#
        )
    );
}

fn start_non_tool_shapes() {
    let tool_without_input = get_content_block_start(&chunk(
        r#"{"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"t2","name":"g"}}"#,
    ))
    .unwrap();
    assert_eq!(tool_without_input.shape, ContentBlockStartShape::ToolUse);
    assert_eq!(tool_without_input.content_block.input, None);
    let server_tool = get_content_block_start(&chunk(
        r#"{"type":"content_block_start","index":3,"content_block":{"type":"server_tool_use","id":"s1","name":"web_search","input":{"query":"q"}}}"#,
    ))
    .unwrap();
    assert_eq!(server_tool.shape, ContentBlockStartShape::Text);
    let thinking = get_content_block_start(&chunk(
        r#"{"type":"content_block_start","index":4,"content_block":{"type":"thinking","thinking":"x","signature":"y"}}"#,
    ))
    .unwrap();
    assert_eq!(thinking.shape, ContentBlockStartShape::Text);
    let text_without_index = get_content_block_start(&chunk(
        r#"{"type":"content_block_start","content_block":{"type":"text","text":"hello"}}"#,
    ))
    .unwrap();
    assert_eq!(text_without_index.shape, ContentBlockStartShape::Text);
    assert_eq!(text_without_index.index, None);
    assert_eq!(
        text_without_index.content_block.text.as_deref(),
        Some("hello")
    );
}

fn redacted_projection() {
    let redacted = get_content_block_start(&chunk(
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"redacted_thinking","data":"opaque"}}"#,
    ))
    .unwrap();
    let mut fields = JsonFields::new();
    let blocks = redacted_thinking_content(&redacted, &mut fields).unwrap();
    assert_eq!(
        as_json(&blocks),
        expected(r#"[{"type":"redacted_thinking","data":"opaque"}]"#)
    );
    assert_eq!(
        as_json(&fields),
        expected(r#"{"thinking_blocks":[{"type":"redacted_thinking","data":"opaque"}]}"#)
    );
}

fn redacted_missing_data() {
    let missing = get_content_block_start(&chunk(
        r#"{"type":"content_block_start","index":5,"content_block":{"type":"redacted_thinking"}}"#,
    ))
    .unwrap();
    assert!(matches!(
        redacted_thinking_content(&missing, &mut JsonFields::new()),
        Err(StreamStateError::FieldType { field: "data" })
    ));
}

pub(super) fn redacted_thinking() {
    redacted_projection();
    start_tool_use();
    start_non_tool_shapes();
    redacted_missing_data();
}

fn usage_first_merges(state: &mut StreamState, counter: &RecordingCounter) {
    let first = handle_usage(
        state,
        &chunk(r#"{"input_tokens":10,"output_tokens":1}"#),
        counter,
    )
    .unwrap();
    assert_eq!(first.prompt_tokens, 10);
    assert_eq!(first.completion_tokens, 1);
    assert_eq!(first.total_tokens, 11);
    assert_eq!(first.speed.as_deref(), Some("fast"));
    assert_eq!(state.cumulative_usage["input_tokens"], chunk("10"));
    let second = handle_usage(state, &chunk(r#"{"output_tokens":5}"#), counter).unwrap();
    assert_eq!(second.prompt_tokens, 10);
    assert_eq!(second.completion_tokens, 5);
    assert_eq!(state.cumulative_usage["input_tokens"], chunk("10"));
}

fn usage_explicit_null(state: &mut StreamState, counter: &RecordingCounter) {
    let usage = handle_usage(state, &chunk(r#"{"input_tokens":null}"#), counter).unwrap();
    assert_eq!(usage.prompt_tokens, 0);
    assert_eq!(state.cumulative_usage["input_tokens"], Json::Null);
}

fn usage_nested_replacement(state: &mut StreamState, counter: &RecordingCounter) {
    let reported = handle_usage(
        state,
        &chunk(r#"{"output_tokens_details":{"thinking_tokens":2}}"#),
        counter,
    )
    .unwrap();
    assert_eq!(reported.completion_tokens_details.reasoning, Some(2));
    assert_eq!(reported.completion_tokens_details.text, Some(3));
    let replaced = handle_usage(state, &chunk(r#"{"output_tokens_details":{}}"#), counter).unwrap();
    assert_eq!(replaced.completion_tokens_details.reasoning, Some(0));
    assert_eq!(replaced.completion_tokens_details.text, Some(5));
    assert_eq!(
        state.cumulative_usage["output_tokens_details"],
        Json::Object(JsonFields::new())
    );
    let empty = handle_usage(state, &chunk("{}"), counter).unwrap();
    assert_eq!(empty.total_tokens, 5);
}

fn usage_reasoning_and_speed(state: &mut StreamState, counter: &RecordingCounter) {
    delta(
        state,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hello"}}"#,
    );
    delta(
        state,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":" world"}}"#,
    );
    let estimated = handle_usage(
        state,
        &chunk(r#"{"input_tokens":3,"output_tokens":10}"#),
        counter,
    )
    .unwrap();
    assert_eq!(estimated.prompt_tokens, 3);
    assert_eq!(estimated.completion_tokens_details.reasoning, Some(2));
    assert_eq!(estimated.completion_tokens_details.text, Some(8));
    assert_eq!(counter.texts(), ["hello world"]);
    let slow = handle_usage(
        state,
        &chunk(r#"{"output_tokens":4,"speed":"slow"}"#),
        counter,
    )
    .unwrap();
    assert_eq!(slow.speed.as_deref(), Some("slow"));
    assert_eq!(state.speed.as_deref(), Some("slow"));
    assert_eq!(counter.texts(), ["hello world", "hello world"]);
}

pub(super) fn usage_start_and_delta() {
    let counter = RecordingCounter::new(2);
    let mut state = state_with(Some(false), Some("fast"));
    usage_first_merges(&mut state, &counter);
    usage_explicit_null(&mut state, &counter);
    usage_nested_replacement(&mut state, &counter);
    usage_reasoning_and_speed(&mut state, &counter);
}

fn web_search_flow(state: &mut StreamState) {
    state.web_search_calls.insert(
        "s1".to_owned(),
        chunk(
            r#"{"id":"ws_s1","type":"web_search_call","status":"in_progress","action":{"type":"search","query":"old","queries":["old"],"sources":[]}}"#,
        ),
    );
    state
        .server_tool_inputs
        .insert("s1".to_owned(), chunk(r#"{"query":"rust async"}"#));
    assert_eq!(
        as_json(&web_search_call_snapshot(state)),
        expected(
            r#"{"s1":{"id":"ws_s1","type":"web_search_call","status":"in_progress","action":{"type":"search","query":"old","queries":["old"],"sources":[]}}}"#
        )
    );
    complete_web_search_call(
        state,
        &chunk(
            r#"{"type":"web_search_tool_result","tool_use_id":"s1","content":[{"type":"web_search_result","url":"https://a.example"}]}"#,
        ),
    );
    assert_eq!(
        as_json(&state.web_search_calls),
        expected(
            r#"{"s1":{"id":"ws_s1","type":"web_search_call","status":"completed","action":{"type":"search","query":"rust async","queries":["rust async"],"sources":[{"type":"url","url":"https://a.example"}]}}}"#
        )
    );
}

fn web_search_untracked(state: &mut StreamState) {
    complete_web_search_call(
        state,
        &chunk(r#"{"type":"web_search_tool_result","tool_use_id":7,"content":[]}"#),
    );
    complete_web_search_call(
        state,
        &chunk(r#"{"type":"web_search_tool_result","tool_use_id":"missing","content":[]}"#),
    );
    complete_web_search_call(
        state,
        &chunk(r#"{"type":"web_search_tool_result","content":[]}"#),
    );
    assert_eq!(state.web_search_calls.len(), 1);
    assert!(state.web_search_calls.contains_key("s1"));
    let mut empty = default_state();
    complete_web_search_call(
        &mut empty,
        &chunk(r#"{"type":"web_search_tool_result","tool_use_id":"ghost","content":[]}"#),
    );
    assert!(empty.web_search_calls.is_empty());
}

pub(super) fn web_search_snapshot() {
    let mut state = default_state();
    web_search_flow(&mut state);
    web_search_untracked(&mut state);
}

fn code_first_buffer() {
    let mut state = default_state();
    state.container_id = Some("c-1".to_owned());
    state.server_tool_inputs = IndexMap::from([
        ("c1".to_owned(), chunk(r#"{"command":"ls -la"}"#)),
        ("c3".to_owned(), chunk("{}")),
    ]);
    state.tool_results = vec![
        chunk(
            r#"{"type":"bash_code_execution_tool_result","tool_use_id":"c1","content":{"stdout":"out","stderr":"err"}}"#,
        ),
        chunk(r#"{"type":"web_search_tool_result","tool_use_id":"c2","content":[]}"#),
        chunk(r#"{"type":"bash_code_execution_tool_result","tool_use_id":"c3","content":{}}"#),
        chunk(
            r#"{"type":"bash_code_execution_tool_result","tool_use_id":"c4","content":{"stdout":"only"}}"#,
        ),
    ];
    assert_eq!(
        as_json(&build_code_interpreter_results(&state).unwrap()),
        expected(
            r#"[{"type":"code_interpreter_call","id":"c1","code":"ls -la","container_id":"c-1","status":"completed","outputs":[{"type":"logs","logs":"outSTDERR: err"}]},{"type":"code_interpreter_call","id":"c3","code":"","container_id":"c-1","status":"completed","outputs":null},{"type":"code_interpreter_call","id":"c4","code":"","container_id":"c-1","status":"completed","outputs":[{"type":"logs","logs":"only"}]}]"#
        )
    );
}

fn code_non_string_command(state: &mut StreamState) {
    state.server_tool_inputs = IndexMap::from([
        ("c1".to_owned(), chunk(r#"{"command":"ls -la"}"#)),
        ("c4".to_owned(), chunk(r#"{"command":5}"#)),
    ]);
    state.tool_results = vec![
        chunk(
            r#"{"type":"bash_code_execution_tool_result","tool_use_id":"c1","content":{"stdout":"out","stderr":"err"}}"#,
        ),
        chunk(r#"{"type":"bash_code_execution_tool_result","tool_use_id":"c4","content":{}}"#),
    ];
    assert!(matches!(
        build_code_interpreter_results(state),
        Err(StreamStateError::FieldType { field: "command" })
    ));
}

fn code_edge_cases(state: &mut StreamState) {
    state.tool_results = vec![chunk(
        r#"{"type":"bash_code_execution_tool_result","content":{}}"#,
    )];
    assert_eq!(
        as_json(&build_code_interpreter_results(state).unwrap()),
        expected(
            r#"[{"type":"code_interpreter_call","id":"","code":"","container_id":"c-1","status":"completed","outputs":null}]"#
        )
    );

    state.server_tool_inputs = IndexMap::from([("c5".to_owned(), chunk(r#""not-an-object""#))]);
    state.tool_results = vec![chunk(
        r#"{"type":"bash_code_execution_tool_result","tool_use_id":"c5","content":"raw"}"#,
    )];
    assert_eq!(
        as_json(&build_code_interpreter_results(state).unwrap()),
        expected(
            r#"[{"type":"code_interpreter_call","id":"c5","code":"","container_id":"c-1","status":"completed","outputs":null}]"#
        )
    );

    state.server_tool_inputs = IndexMap::from([("c6".to_owned(), chunk(r#"{"command":""}"#))]);
    state.tool_results = vec![chunk(
        r#"{"type":"bash_code_execution_tool_result","tool_use_id":"c6","content":{"stdout":"out","stderr":""}}"#,
    )];
    assert_eq!(
        as_json(&build_code_interpreter_results(state).unwrap()),
        expected(
            r#"[{"type":"code_interpreter_call","id":"c6","code":"","container_id":"c-1","status":"completed","outputs":[{"type":"logs","logs":"out"}]}]"#
        )
    );

    state.server_tool_inputs = IndexMap::from([("c7".to_owned(), chunk(r#"{"command":null}"#))]);
    state.tool_results = vec![chunk(
        r#"{"type":"bash_code_execution_tool_result","tool_use_id":"c7","content":{}}"#,
    )];
    assert_eq!(
        as_json(&build_code_interpreter_results(state).unwrap()),
        expected(
            r#"[{"type":"code_interpreter_call","id":"c7","code":null,"container_id":"c-1","status":"completed","outputs":null}]"#
        )
    );
}

pub(super) fn code_output_buffer() {
    code_first_buffer();
    let mut state = default_state();
    state.container_id = Some("c-1".to_owned());
    code_non_string_command(&mut state);
    code_edge_cases(&mut state);
}

#[rstest]
#[case::end_turn(
    r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#,
    "stop"
)]
#[case::max_tokens(
    r#"{"type":"message_delta","delta":{"stop_reason":"max_tokens"}}"#,
    "length"
)]
#[case::missing(r#"{"type":"message_delta","delta":{}}"#, "stop")]
#[case::null_reason(r#"{"type":"message_delta","delta":{"stop_reason":null}}"#, "stop")]
#[case::empty_reason(r#"{"type":"message_delta","delta":{"stop_reason":""}}"#, "stop")]
#[case::tool_use(
    r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
    "tool_calls"
)]
fn message_delta_finish_reasons(#[case] source: &str, #[case] finish_reason: &str) {
    let counter = RecordingCounter::new(0);
    let mut state = default_state();
    let projection = handle_message_delta(&mut state, &chunk(source), &counter).unwrap();
    assert_eq!(projection.finish_reason, finish_reason);
}

fn message_delta_usage_container(state: &mut StreamState, counter: &RecordingCounter) {
    let with_usage = handle_message_delta(
        state,
        &chunk(r#"{"type":"message_delta","delta":{},"usage":{"output_tokens":7}}"#),
        counter,
    )
    .unwrap();
    let usage = with_usage.usage.unwrap();
    assert_eq!(usage.prompt_tokens, 0);
    assert_eq!(usage.completion_tokens, 7);
    assert_eq!(with_usage.container, None);
    let with_container = handle_message_delta(
        state,
        &chunk(r#"{"type":"message_delta","delta":{"container":{"id":"c1"}}}"#),
        counter,
    )
    .unwrap();
    assert_eq!(with_container.usage, None);
    assert_eq!(with_container.container, Some(chunk(r#"{"id":"c1"}"#)));
    assert_eq!(state.container_id, None);
}

fn message_delta_converted(state: &mut StreamState, counter: &RecordingCounter) {
    state.converted_response_format_tool = true;
    let converted = handle_message_delta(
        state,
        &chunk(r#"{"type":"message_delta","delta":{"stop_reason":"max_tokens"}}"#),
        counter,
    )
    .unwrap();
    assert_eq!(converted.finish_reason, "stop");
}

fn message_delta_provider_fields(state: &mut StreamState) {
    let citation = delta(
        state,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"citations_delta","citation":{"cited_text":"cite"}}}"#,
    );
    assert_eq!(
        as_json(&citation.provider_specific_fields),
        expected(r#"{"citation":{"cited_text":"cite"}}"#)
    );
    let compaction = delta(
        state,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"compaction_delta","content":{"summary":"keep"},"extra":"keep"}}"#,
    );
    assert_eq!(
        as_json(&compaction.provider_specific_fields),
        expected(
            r#"{"compaction_delta":{"type":"compaction_delta","content":{"summary":"keep"}}}"#
        )
    );
    let ignored = delta(
        state,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","content":{"summary":"ignored"}}}"#,
    );
    assert!(ignored.provider_specific_fields.is_empty());
}

pub(super) fn stop_reason_state() {
    let counter = RecordingCounter::new(0);
    let mut state = default_state();
    message_delta_usage_container(&mut state, &counter);
    message_delta_converted(&mut state, &counter);
    message_delta_provider_fields(&mut state);
}

#[test]
fn json_fragment_buffer_appends_and_replaces() {
    let mut state = default_state();
    assert_eq!(state.json_buffer.snapshot(), "");
    state.json_buffer.append("{\"a\":");
    state.json_buffer.append("1}");
    assert_eq!(state.json_buffer.snapshot(), "{\"a\":1}");
    state.json_buffer.set("tail".to_owned());
    assert_eq!(state.json_buffer.snapshot(), "tail");
    state.json_buffer.append("-more");
    assert_eq!(state.json_buffer.snapshot(), "tail-more");
}

#[test]
fn uuid_v4_source_returns_a_random_id() {
    let source = UuidV4;
    let first = source.next_id();
    assert_ne!(first, Uuid::nil());
    assert_ne!(first, source.next_id());
}
