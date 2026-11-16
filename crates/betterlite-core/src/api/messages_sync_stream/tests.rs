#![cfg(test)]

use std::collections::VecDeque;

use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;
use indexmap::IndexMap;
use uuid::Uuid;

use crate::api::messages_context_clear::result::{
    AppliedEdit, CompactionBlock, CompactionKind, UsageIteration, UsageKind,
};
use crate::api::messages_stream_split::ChunkSource;
use crate::api::messages_stream_split::errors::StreamFailure;
use crate::api::messages_stream_usage::state::{StreamSetup, StreamState};
use crate::api::messages_stream_usage::usage::initial_usage;

use super::driver::SyncDriver;
use super::engine;

const TEXT_START: &str =
    r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#;
const DELTA_A: &str =
    r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"A"}}"#;
const STOP_0: &str = r#"{"type":"content_block_stop","index":0}"#;
const STOP_1: &str = r#"{"type":"content_block_stop","index":1}"#;
const STOP_2: &str = r#"{"type":"content_block_stop","index":2}"#;
const MESSAGE_STOP: &str = r#"{"type":"message_stop"}"#;
const MESSAGE_DELTA: &str = r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"input_tokens":0,"output_tokens":0}}"#;
const HELD_TEXT: &str =
    r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"H"}}"#;
const HELD_STOP: &str = r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"input_tokens":9,"output_tokens":3}}"#;
const THINKING_START: &str = r#"{"type":"content_block_start","index":1,"content_block":{"type":"thinking","thinking":"","signature":""}}"#;
const THINKING_DELTA: &str =
    r#"{"type":"content_block_delta","index":1,"delta":{"type":"thinking_delta","thinking":"R"}}"#;
const TOOL_START: &str = r#"{"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"call_1","name":"lookup","input":{}}}"#;
const TOOL_DELTA: &str = r#"{"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"x\":1}"}}"#;
const TOOL_START_CALL_2: &str = r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"call_2","name":"lookup","input":{}}}"#;
const STOP_WITH_EDITS: &str = r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"input_tokens":5,"output_tokens":2,"iterations":[{"type":"compaction","input_tokens":4,"output_tokens":1},{"type":"message","input_tokens":5,"output_tokens":2}]},"context_management":{"applied_edits":[{"type":"clear_tool_uses_20250919","cleared_tool_uses":1}]}}"#;
const FLUSHED_WITH_EDITS: &str = r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"input_tokens":0,"output_tokens":0,"iterations":[{"type":"compaction","input_tokens":4,"output_tokens":1}]},"context_management":{"applied_edits":[{"type":"clear_tool_uses_20250919","cleared_tool_uses":1}]}}"#;

struct Scripted {
    items: VecDeque<Result<StreamChunk, StreamFailure>>,
    pulls: usize,
}

impl ChunkSource for Scripted {
    fn next_chunk(&mut self) -> Result<Option<StreamChunk>, StreamFailure> {
        self.pulls += 1;
        self.items.pop_front().transpose()
    }
}

struct FixedIds;

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        Uuid::from_u128(7)
    }
}

type Driver = SyncDriver<Scripted, FixedIds>;

fn chunk(source: &str) -> StreamChunk {
    serde_json::from_str(source).expect("a valid chunk fixture")
}

fn object(source: &str) -> Object {
    serde_json::from_str(source).expect("a valid object fixture")
}

fn text(text: &str) -> StreamChunk {
    chunk(&format!(
        r#"{{"choices":[{{"index":0,"delta":{{"content":"{text}"}},"finish_reason":null}}]}}"#
    ))
}

fn finish(reason: &str) -> StreamChunk {
    chunk(&format!(
        r#"{{"choices":[{{"index":0,"delta":{{}},"finish_reason":"{reason}"}}]}}"#
    ))
}

fn usage_chunk(prompt: i64, completion: i64) -> StreamChunk {
    chunk(&format!(
        r#"{{"choices":[],"usage":{{"prompt_tokens":{prompt},"completion_tokens":{completion}}}}}"#
    ))
}

fn setup() -> StreamSetup {
    StreamSetup {
        model: "claude-sonnet-4-5".to_owned(),
        tool_name_mapping: IndexMap::new(),
        applied_edits: Vec::new(),
        compaction_block: None,
        iterations_usage: None,
    }
}

fn polyfilled() -> StreamSetup {
    StreamSetup {
        applied_edits: vec![AppliedEdit {
            edit_type: Some("clear_tool_uses_20250919".to_owned()),
            cleared_input_tokens: None,
            cleared_tool_uses: Some(1),
            cleared_thinking_turns: None,
            summary_input_tokens: None,
            summary_output_tokens: None,
            error: None,
            warnings: None,
        }],
        iterations_usage: Some(vec![UsageIteration {
            kind: UsageKind::Compaction,
            input_tokens: Some(4),
            output_tokens: Some(1),
        }]),
        ..setup()
    }
}

fn driver(chunks: Vec<StreamChunk>) -> Driver {
    started(chunks, setup())
}

fn started(chunks: Vec<StreamChunk>, setup: StreamSetup) -> Driver {
    let source = Scripted {
        items: chunks.into_iter().map(Ok).collect(),
        pulls: 0,
    };
    SyncDriver {
        state: StreamState::new(None, source, setup),
        ids: FixedIds,
    }
}

fn next(driver: &mut Driver) -> Object {
    driver.next_event().expect("the stream still has events")
}

fn take(driver: &mut Driver, count: usize) -> Vec<Object> {
    (0..count).map(|_| next(driver)).collect()
}

fn kind(event: &Object) -> &str {
    event
        .get("type")
        .and_then(Json::as_str)
        .expect("every event names its type")
}

fn assert_events(driver: &mut Driver, frames: &[&str]) {
    let expected: Vec<Object> = frames.iter().copied().map(object).collect();
    let events = take(driver, expected.len());
    assert_eq!(events, expected);
}

fn assert_message(start: &Object) {
    assert_eq!(kind(start), "message_start");
    let announced = start
        .get("message")
        .and_then(Json::as_object)
        .expect("message_start carries the message");
    assert_eq!(announced.get("content"), Some(&Json::List(Vec::new())));
    assert_eq!(announced.get("stop_reason"), Some(&Json::Null));
    assert_eq!(announced.get("stop_sequence"), Some(&Json::Null));
    assert_eq!(
        announced.get("model").and_then(Json::as_str),
        Some("claude-sonnet-4-5")
    );
    assert_eq!(announced.get("usage"), Some(&Json::Object(initial_usage())));
    let id = announced
        .get("id")
        .and_then(Json::as_str)
        .expect("the message names its id");
    assert!(id.starts_with("msg_"), "a fresh msg_ uuid");
}

#[test]
fn initial_events_precede_provider_pulls() {
    let mut stream = driver(vec![text("A")]);
    assert_message(&next(&mut stream));
    assert_eq!(stream.state.completion_stream.source.pulls, 0);
    assert_events(&mut stream, &[TEXT_START, DELTA_A]);
    assert_eq!(
        stream.state.completion_stream.source.pulls, 1,
        "the start waited for one chunk and the delta came from the queue"
    );
    assert_events(&mut stream, &[MESSAGE_STOP]);
}

#[test]
fn compaction_frames_precede_every_pull() {
    let block = CompactionBlock {
        kind: CompactionKind::Compaction,
        content: Some("S".to_owned()),
        signature: None,
    };
    let mut stream = started(
        vec![text("A")],
        StreamSetup {
            compaction_block: Some(block),
            ..setup()
        },
    );
    assert_message(&next(&mut stream));
    assert_events(
        &mut stream,
        &[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"compaction","content":""}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"compaction_delta","content":"S"}}"#,
            STOP_0,
        ],
    );
    assert_eq!(
        stream.state.completion_stream.source.pulls, 0,
        "the synthetic block needs no read"
    );
    assert_events(
        &mut stream,
        &[r#"{"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}"#],
    );
    assert_eq!(stream.state.completion_stream.source.pulls, 1);
}

#[test]
fn text_thinking_tool_transitions() {
    let role =
        chunk(r#"{"choices":[{"index":0,"delta":{"role":"assistant"},"finish_reason":null}]}"#);
    let reasoning = chunk(
        r#"{"choices":[{"index":0,"delta":{"reasoning_content":"R"},"finish_reason":null}]}"#,
    );
    let tool = chunk(
        r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"lookup","arguments":"{\"x\":1}"}}]},"finish_reason":null}]}"#,
    );
    let mut stream = driver(vec![role, text("A"), reasoning, tool, finish("stop")]);
    assert_message(&next(&mut stream));
    assert_events(&mut stream, &[TEXT_START, DELTA_A]);
    assert_eq!(
        stream.state.completion_stream.source.pulls, 2,
        "the role-only chunk opened nothing"
    );
    assert_events(
        &mut stream,
        &[
            STOP_0,
            THINKING_START,
            THINKING_DELTA,
            STOP_1,
            TOOL_START,
            TOOL_DELTA,
            STOP_2,
            MESSAGE_DELTA,
            MESSAGE_STOP,
        ],
    );
}

#[test]
fn a_named_tool_with_empty_arguments_starts_at_once() {
    let tool = chunk(
        r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_2","type":"function","function":{"name":"lookup","arguments":""}}]},"finish_reason":null}]}"#,
    );
    let mut stream = driver(vec![tool]);
    assert_message(&next(&mut stream));
    assert_events(&mut stream, &[TOOL_START_CALL_2]);
    assert_eq!(
        stream.state.completion_stream.source.pulls, 1,
        "the empty delta flushed the start"
    );
}

#[test]
fn an_empty_unsigned_thinking_delta_opens_nothing() {
    let thinking = chunk(
        r#"{"choices":[{"index":0,"delta":{"thinking_blocks":[{"type":"thinking","thinking":""}]},"finish_reason":null}]}"#,
    );
    let mut stream = driver(vec![thinking, text("A")]);
    assert_message(&next(&mut stream));
    assert_events(&mut stream, &[TEXT_START]);
    assert_eq!(stream.state.completion_stream.source.pulls, 2);
}

#[test]
fn combined_content_and_finish_are_split() {
    let combined =
        chunk(r#"{"choices":[{"index":0,"delta":{"content":"C"},"finish_reason":"stop"}]}"#);
    let mut stream = driver(vec![combined]);
    assert_message(&next(&mut stream));
    assert_events(
        &mut stream,
        &[
            TEXT_START,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"C"}}"#,
            STOP_0,
            MESSAGE_DELTA,
        ],
    );
}

#[test]
fn finish_waits_for_usage() {
    let mut stream = started(
        vec![text("A"), finish("stop"), usage_chunk(5, 2)],
        polyfilled(),
    );
    assert_message(&next(&mut stream));
    assert_events(
        &mut stream,
        &[TEXT_START, DELTA_A, STOP_0, STOP_WITH_EDITS, MESSAGE_STOP],
    );
}

#[test]
fn a_usage_bearing_choice_chunk_merges_into_a_held_stop() {
    let with_usage = chunk(
        r#"{"choices":[{"index":0,"delta":{"content":"B"},"finish_reason":null}],"usage":{"prompt_tokens":7,"completion_tokens":3}}"#,
    );
    let mut stream = driver(vec![text("A"), finish("stop"), with_usage]);
    assert_message(&next(&mut stream));
    assert_events(
        &mut stream,
        &[
            TEXT_START,
            DELTA_A,
            STOP_0,
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"input_tokens":7,"output_tokens":3}}"#,
        ],
    );
}

#[test]
fn usage_stops_the_stream_after_the_final_message_delta() {
    let mut stream = driver(vec![
        text("A"),
        finish("stop"),
        usage_chunk(5, 2),
        text("Z"),
        finish("stop"),
    ]);
    assert_message(&next(&mut stream));
    assert_events(
        &mut stream,
        &[
            TEXT_START,
            DELTA_A,
            STOP_0,
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"input_tokens":5,"output_tokens":2}}"#,
        ],
    );
    assert_events(&mut stream, &[MESSAGE_STOP]);
    assert!(stream.next_event().is_none(), "the stream has ended once");
}

#[test]
fn finish_without_usage_flushes() {
    let mut stream = started(vec![text("A"), finish("stop")], polyfilled());
    assert_message(&next(&mut stream));
    assert_events(
        &mut stream,
        &[
            TEXT_START,
            DELTA_A,
            STOP_0,
            FLUSHED_WITH_EDITS,
            MESSAGE_STOP,
        ],
    );
    assert!(stream.next_event().is_none());
}

#[test]
fn an_unclosed_block_is_stopped_before_the_flushed_delta() {
    let mut stream = driver(Vec::new());
    stream.state.message_progress.sent_first_chunk = true;
    stream.state.block_progress.sent_content_block_start = true;
    stream.state.holding_stop_reason_chunk = Some(object(HELD_STOP));
    assert_events(&mut stream, &[STOP_0, HELD_STOP, MESSAGE_STOP]);
    assert!(stream.next_event().is_none());
}

#[test]
fn a_pure_eof_emits_only_the_message_start_and_stop() {
    let mut stream = driver(Vec::new());
    assert_message(&next(&mut stream));
    assert_events(&mut stream, &[MESSAGE_STOP]);
    assert_eq!(stream.state.completion_stream.source.pulls, 1);
    assert!(stream.next_event().is_none());
}

#[test]
fn a_finish_only_chunk_opens_and_closes_an_empty_text_block() {
    let mut stream = driver(vec![finish("stop")]);
    assert_message(&next(&mut stream));
    assert_events(
        &mut stream,
        &[TEXT_START, STOP_0, MESSAGE_DELTA, MESSAGE_STOP],
    );
}

#[test]
fn held_content_comes_before_the_flushed_delta() {
    let mut stream = driver(Vec::new());
    stream.state.message_progress.sent_first_chunk = true;
    stream.state.holding_chunk = Some(object(HELD_TEXT));
    stream.state.holding_stop_reason_chunk = Some(object(HELD_STOP));
    assert_events(&mut stream, &[HELD_TEXT, STOP_0, HELD_STOP, MESSAGE_STOP]);
}

#[test]
fn queued_usage_drops_the_held_content() {
    let mut stream = driver(Vec::new());
    stream.state.message_progress.sent_first_chunk = true;
    stream.state.queued_usage_chunk = true;
    stream.state.holding_chunk = Some(object(HELD_TEXT));
    assert_events(&mut stream, &[MESSAGE_STOP]);
    assert!(stream.state.holding_chunk.is_none());
}

#[test]
fn an_existing_held_chunk_precedes_the_processed_event() {
    let mut stream = driver(vec![text("A")]);
    stream.state.message_progress.sent_first_chunk = true;
    stream.state.holding_chunk = Some(object(HELD_TEXT));
    assert_events(&mut stream, &[TEXT_START, HELD_TEXT, DELTA_A]);
    assert!(stream.state.holding_chunk.is_none());
}

#[test]
fn the_stop_iteration_path_flushes_queued_events_before_the_held_stop() {
    let mut stream = driver(Vec::new());
    stream.state.message_progress.sent_first_chunk = true;
    stream.state.block_progress.sent_content_block_finish = true;
    let queued = object(MESSAGE_STOP);
    stream.state.chunk_queue.push_back(queued.clone());
    stream.state.holding_stop_reason_chunk = Some(object(HELD_STOP));
    let state = &mut stream.state;
    assert_eq!(
        engine::stop_iteration(state).expect("the queued event"),
        Some(queued)
    );
    assert_eq!(
        engine::stop_iteration(state).expect("the held delta"),
        Some(object(HELD_STOP))
    );
    assert!(state.holding_stop_reason_chunk.is_none());
    assert_eq!(
        engine::stop_iteration(state).expect("the last message"),
        Some(object(MESSAGE_STOP))
    );
    assert_eq!(engine::stop_iteration(state).expect("nothing held"), None);
}

#[test]
fn the_stop_iteration_path_closes_an_open_block_first() {
    let mut stream = driver(Vec::new());
    stream.state.block_progress.sent_content_block_start = true;
    stream.state.holding_stop_reason_chunk = Some(object(HELD_STOP));
    let state = &mut stream.state;
    assert_eq!(
        engine::stop_iteration(state).expect("the block stop"),
        Some(object(STOP_0))
    );
    assert!(state.block_progress.sent_content_block_finish);
    assert_eq!(
        engine::stop_iteration(state).expect("the held delta"),
        Some(object(HELD_STOP))
    );
    assert!(state.holding_stop_reason_chunk.is_none());
}

#[test]
fn sync_stream_failure_ends_iteration() {
    let source = Scripted {
        items: VecDeque::from(vec![
            Ok(text("A")),
            Err(StreamFailure::Unclassified {
                text: "the provider chunk is null".to_owned(),
            }),
            Err(StreamFailure::Reported {
                status_code: 503,
                message: "the provider is unavailable".to_owned(),
            }),
        ]),
        pulls: 0,
    };
    let mut stream = SyncDriver {
        state: StreamState::new(None, source, setup()),
        ids: FixedIds,
    };
    assert_message(&next(&mut stream));
    assert_events(&mut stream, &[TEXT_START, DELTA_A]);
    assert!(
        stream.next_event().is_none(),
        "an unclassified failed pull ends the iteration instead of raising"
    );
    assert!(
        stream.next_event().is_none(),
        "a reported failed pull ends the iteration too"
    );
}
