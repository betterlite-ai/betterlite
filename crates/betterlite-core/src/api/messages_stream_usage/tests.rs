#![cfg(test)]

use std::time::Duration;

use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json as ModelJson;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;
use chrono::DateTime;
use indexmap::IndexMap;
use rstest::rstest;
use uuid::Uuid;

use crate::api::messages_chat_output::records::{ChoiceRecord, UsageRecord};
use crate::api::messages_context_clear::result::{
    AppliedEdit, CompactionBlock, CompactionKind, UsageIteration, UsageKind,
};
use crate::api::messages_stream_split::ChunkSource;
use crate::api::messages_stream_split::errors::StreamFailure;

use super::blocks::{
    delta_has_content, is_blank_delta, next_compaction_event, should_start_new_content_block,
    with_refusal_stop_details,
};
use super::error::StreamStateError;
use super::state::{BlockKind, StreamSetup, StreamState};
use super::usage::{
    augment_message_delta_usage, handle_choiceless_chunk, initial_usage, merge_usage_into_held_stop,
};

const PROBE_USAGE: &str =
    r#"{"prompt_tokens":8,"completion_tokens":2,"cache_read_input_tokens":3}"#;

struct PassThrough;

impl ChunkSource for PassThrough {
    fn next_chunk(&mut self) -> Result<Option<StreamChunk>, StreamFailure> {
        Ok(None)
    }
}

struct Forwarding {
    chunks: Option<Vec<StreamChunk>>,
    messages: Option<Vec<ModelJson>>,
}

impl ChunkSource for Forwarding {
    fn next_chunk(&mut self) -> Result<Option<StreamChunk>, StreamFailure> {
        Ok(None)
    }

    fn chunks(&self) -> Option<&[StreamChunk]> {
        self.chunks.as_deref()
    }

    fn messages(&self) -> Option<&[ModelJson]> {
        self.messages.as_deref()
    }
}

struct FixedIds;

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        Uuid::from_u128(7)
    }
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

fn state() -> StreamState<PassThrough> {
    StreamState::new(None, PassThrough, setup())
}

fn state_with(setup: StreamSetup) -> StreamState<PassThrough> {
    StreamState::new(None, PassThrough, setup)
}

fn object(source: &str) -> Object {
    serde_json::from_str(source).expect("valid object fixture")
}

fn json_object(source: &str) -> Json {
    Json::Object(object(source))
}

fn choice(source: &str) -> ChoiceRecord {
    serde_json::from_str(source).expect("valid choice fixture")
}

fn record(source: &str) -> UsageRecord {
    serde_json::from_str(source).expect("valid usage fixture")
}

fn start<T>(state: &mut StreamState<T>, source: &str) -> bool
where
    T: ChunkSource,
{
    should_start_new_content_block(state, &[choice(source)], &FixedIds).expect("a single choice")
}

fn applied_edit() -> AppliedEdit {
    AppliedEdit {
        edit_type: Some("clear_tool_uses_20250919".to_owned()),
        cleared_input_tokens: None,
        cleared_tool_uses: Some(1),
        cleared_thinking_turns: None,
        summary_input_tokens: None,
        summary_output_tokens: None,
        error: None,
        warnings: None,
    }
}

fn compaction_block(content: Option<&str>) -> CompactionBlock {
    CompactionBlock {
        kind: CompactionKind::Compaction,
        content: content.map(ToOwned::to_owned),
        signature: None,
    }
}

fn compaction_state(content: Option<&str>) -> StreamState<PassThrough> {
    state_with(StreamSetup {
        compaction_block: Some(compaction_block(content)),
        ..setup()
    })
}

fn summary_iteration() -> UsageIteration {
    UsageIteration {
        kind: UsageKind::Compaction,
        input_tokens: Some(12),
        output_tokens: Some(3),
    }
}

fn iterations_state() -> StreamState<PassThrough> {
    state_with(StreamSetup {
        iterations_usage: Some(vec![summary_iteration()]),
        ..setup()
    })
}

fn logging_state() -> LoggingState {
    let mut context = CorrelationContext::default();
    LoggingState::new(
        LoggingInit {
            model: "claude".to_owned(),
            call_type: "anthropic_messages".to_owned(),
            start_time: DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00")
                .expect("the fixture time is RFC 3339"),
            litellm_call_id: "call-1".to_owned(),
            function_id: "fn-1".to_owned(),
            messages: None,
            stream: None,
            litellm_trace_id: Some("trace-1".to_owned()),
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles"),
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .expect("the logging fixture builds")
}

#[test]
fn stream_state_initialization() {
    let state = state();
    assert_eq!(state.current_content_block_index, 0);
    assert_eq!(state.current_content_block_type, BlockKind::Text);
    assert_eq!(
        state.current_content_block_start,
        json_object(r#"{"type": "text", "text": ""}"#)
    );
    assert!(state.chunk_queue.is_empty());
    assert!(state.refusal_text.is_empty());
    assert!(state.tool_name_mapping.is_empty());
    assert!(state.applied_edits.is_empty());
    assert!(state.compaction_block.is_none());
    assert!(state.iterations_usage.is_none());
    assert!(state.holding_chunk.is_none());
    assert!(state.holding_stop_reason_chunk.is_none());
}

#[test]
fn stream_state_flags_start_clear() {
    let state = state();
    assert!(!state.message_progress.sent_first_chunk);
    assert!(!state.message_progress.sent_last_message);
    assert!(!state.block_progress.sent_content_block_start);
    assert!(!state.block_progress.sent_content_block_finish);
    assert!(!state.compaction_progress.sent_compaction_block_start);
    assert!(!state.compaction_progress.sent_compaction_block_delta);
    assert!(!state.queued_usage_chunk);
    assert!(!state.sent_compaction_block);
}

#[test]
fn stream_state_names_the_message_id_after_a_uuid() {
    let state = state();
    assert!(state.message_id.starts_with("msg_"));
    let id = state
        .message_id
        .strip_prefix("msg_")
        .expect("the id prefix");
    assert_eq!(
        Uuid::parse_str(id)
            .expect("the id suffix is a uuid")
            .get_version_num(),
        4
    );
}

#[test]
fn initial_usage_is_four_zero_counters() {
    let usage = initial_usage();
    assert_eq!(usage.len(), 4);
    assert_eq!(
        usage,
        object(
            r#"{"input_tokens": 0, "output_tokens": 0,
                "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0}"#
        )
    );
}

#[test]
fn independent_instances_share_nothing() {
    let mut first = state();
    let second = state();
    assert_ne!(first.message_id, second.message_id);
    first.chunk_queue.push_back(object(r#"{"type": "queued"}"#));
    first.current_content_block_start = Json::Text("mutated".to_owned());
    assert!(second.chunk_queue.is_empty());
    assert_eq!(
        second.current_content_block_start,
        json_object(r#"{"type": "text", "text": ""}"#)
    );
}

#[test]
fn constructor_records_the_message_id_on_logging() {
    let mut logging = logging_state();
    assert!(logging.streamed_anthropic_message_id.is_none());
    let state = StreamState::new(Some(&mut logging), PassThrough, setup());
    assert_eq!(
        logging.streamed_anthropic_message_id.as_deref(),
        Some(state.message_id.as_str())
    );
}

#[test]
fn chunks_and_messages_forward_through_the_splitter() {
    let messages = vec![
        serde_json::from_str(r#"{"role": "user", "content": "hi"}"#)
            .expect("valid message fixture"),
    ];
    let state = StreamState::new(
        None,
        Forwarding {
            chunks: Some(vec![StreamChunk::default(), StreamChunk::default()]),
            messages: Some(messages.clone()),
        },
        setup(),
    );
    assert_eq!(
        state.completion_stream.chunks().map(<[StreamChunk]>::len),
        Some(2)
    );
    assert_eq!(
        state.completion_stream.messages(),
        Some(messages.as_slice())
    );
    let empty = StreamState::new(
        None,
        Forwarding {
            chunks: None,
            messages: None,
        },
        setup(),
    );
    assert!(empty.completion_stream.chunks().is_none());
    assert!(empty.completion_stream.messages().is_none());
}

#[test]
fn applied_edits_are_copied_into_the_state() {
    let state = state_with(StreamSetup {
        applied_edits: vec![applied_edit()],
        ..setup()
    });
    assert_eq!(state.applied_edits, vec![applied_edit()]);
}

#[test]
fn held_usage_replaces_placeholder() {
    let mut state = state();
    state.holding_stop_reason_chunk = Some(object(
        r#"{"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"input_tokens": 0, "output_tokens": 0}}"#,
    ));
    let merged = merge_usage_into_held_stop(&state, &record(PROBE_USAGE)).expect("a held stop");
    assert_eq!(
        merged,
        object(
            r#"{"type": "message_delta", "delta": {"stop_reason": "end_turn"},
                "usage": {"input_tokens": 5, "output_tokens": 2, "cache_read_input_tokens": 3}}"#
        )
    );
}

#[test]
fn held_usage_adds_a_missing_delta() {
    let mut state = state();
    state.holding_stop_reason_chunk = Some(object(r#"{"type": "message_delta"}"#));
    let merged = merge_usage_into_held_stop(&state, &record(PROBE_USAGE)).expect("a held stop");
    assert_eq!(merged.get("delta"), Some(&Json::Object(Object::new())));
}

#[test]
fn held_usage_attaches_context_edits() {
    let mut state = state_with(StreamSetup {
        applied_edits: vec![applied_edit()],
        ..setup()
    });
    state.holding_stop_reason_chunk = Some(object(r#"{"type": "message_delta"}"#));
    let merged = merge_usage_into_held_stop(&state, &record(PROBE_USAGE)).expect("a held stop");
    let attached = merged.get("context_management").expect("attached");
    assert_eq!(
        attached,
        &json_object(
            r#"{"applied_edits": [{"type": "clear_tool_uses_20250919", "cleared_tool_uses": 1}]}"#
        )
    );
}

#[test]
fn held_usage_keeps_an_existing_context_management() {
    let mut state = state_with(StreamSetup {
        applied_edits: vec![applied_edit()],
        ..setup()
    });
    state.holding_stop_reason_chunk = Some(object(
        r#"{"type": "message_delta", "context_management": null}"#,
    ));
    let merged = merge_usage_into_held_stop(&state, &record(PROBE_USAGE)).expect("a held stop");
    assert_eq!(merged.get("context_management"), Some(&Json::Null));
    state.holding_stop_reason_chunk = Some(object(
        r#"{"type": "message_delta", "context_management": {"applied_edits": []}}"#,
    ));
    let retained_chunk =
        merge_usage_into_held_stop(&state, &record(PROBE_USAGE)).expect("a held stop");
    let retained = retained_chunk.get("context_management").expect("retained");
    assert_eq!(retained, &json_object(r#"{"applied_edits": []}"#));
}

#[test]
fn choiceless_chunk_queues_merged_usage() {
    let held = object(r#"{"type": "message_delta", "delta": {"stop_reason": "end_turn"}}"#);
    let probe = record(PROBE_USAGE);
    let mut expected_state = state();
    expected_state.holding_stop_reason_chunk = Some(held.clone());
    let expected = merge_usage_into_held_stop(&expected_state, &probe).expect("a held stop");
    let mut state = state();
    state.holding_stop_reason_chunk = Some(held);
    assert!(handle_choiceless_chunk(&mut state, Some(&probe)).expect("a usage"));
    assert_eq!(state.chunk_queue.len(), 1);
    assert_eq!(state.chunk_queue.front(), Some(&expected));
    assert!(state.queued_usage_chunk);
    assert!(state.holding_stop_reason_chunk.is_none());
}

#[test]
fn held_usage_gets_polyfill_iterations() {
    let held = object(
        r#"{"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"input_tokens": 0, "output_tokens": 0}}"#,
    );
    let expected = object(
        r#"{"type": "message_delta", "delta": {"stop_reason": "end_turn"},
            "usage": {"input_tokens": 5, "output_tokens": 2, "cache_read_input_tokens": 3,
                      "iterations": [
                          {"type": "compaction", "input_tokens": 12, "output_tokens": 3},
                          {"type": "message", "input_tokens": 5, "output_tokens": 2}
                      ]}}"#,
    );
    let mut state = iterations_state();
    state.holding_stop_reason_chunk = Some(held);
    let merged = merge_usage_into_held_stop(&state, &record(PROBE_USAGE)).expect("a held stop");
    assert_eq!(merged, expected);
    assert!(handle_choiceless_chunk(&mut state, Some(&record(PROBE_USAGE))).expect("a usage"));
    assert_eq!(state.chunk_queue.front(), Some(&expected));
}

#[test]
fn choiceless_chunk_requires_a_held_stop() {
    let probe = record(PROBE_USAGE);
    let mut state = state();
    assert!(!handle_choiceless_chunk(&mut state, Some(&probe)).expect("no held"));
    assert!(state.chunk_queue.is_empty());
    assert!(!state.queued_usage_chunk);
    state.holding_stop_reason_chunk = Some(object(r#"{"type": "message_delta"}"#));
    assert!(!handle_choiceless_chunk(&mut state, None).expect("no usage"));
    assert!(state.chunk_queue.is_empty());
    assert!(!state.queued_usage_chunk);
    assert!(state.holding_stop_reason_chunk.is_some());
}

#[test]
fn held_usage_merge_without_held_stop_is_an_error() {
    let state = state();
    assert!(matches!(
        merge_usage_into_held_stop(&state, &record(PROBE_USAGE)),
        Err(StreamStateError::MissingHeldStop)
    ));
}

#[test]
fn compaction_usage_iterations() {
    let state = iterations_state();
    let augmented = augment_message_delta_usage(
        &state,
        object(
            r#"{"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"input_tokens": 5, "output_tokens": 2}}"#,
        ),
    )
    .expect("augmented");
    assert_eq!(
        augmented,
        object(
            r#"{"type": "message_delta", "delta": {"stop_reason": "end_turn"},
                "usage": {"input_tokens": 5, "output_tokens": 2,
                          "iterations": [
                              {"type": "compaction", "input_tokens": 12, "output_tokens": 3},
                              {"type": "message", "input_tokens": 5, "output_tokens": 2}
                          ]}}"#
        )
    );
}

#[test]
fn zero_counters_keep_only_summary_iterations() {
    let state = iterations_state();
    let augmented = augment_message_delta_usage(
        &state,
        object(r#"{"type": "message_delta", "usage": {"input_tokens": 0, "output_tokens": 0}}"#),
    )
    .expect("augmented");
    let usage = augmented.get("usage").expect("usage");
    assert_eq!(
        usage,
        &json_object(
            r#"{"input_tokens": 0, "output_tokens": 0,
                "iterations": [{"type": "compaction", "input_tokens": 12, "output_tokens": 3}]}"#
        )
    );
}

#[rstest]
#[case::input_only(
    r#"{"type": "message_delta", "usage": {"input_tokens": 5, "output_tokens": 0}}"#,
    5,
    0
)]
#[case::output_only(
    r#"{"type": "message_delta", "usage": {"input_tokens": 0, "output_tokens": 2}}"#,
    0,
    2
)]
fn single_sided_counters_still_append_a_message_iteration(
    #[case] event: &str,
    #[case] input: i64,
    #[case] output: i64,
) {
    let state = iterations_state();
    let augmented = augment_message_delta_usage(&state, object(event)).expect("augmented");
    let expected = json_object(&format!(
        r#"{{"input_tokens": {input}, "output_tokens": {output}, "iterations": [{{"type": "compaction", "input_tokens": 12, "output_tokens": 3}}, {{"type": "message", "input_tokens": {input}, "output_tokens": {output}}}]}}"#
    ));
    assert_eq!(augmented.get("usage"), Some(&expected));
}

#[test]
fn missing_counters_keep_only_summary_iterations() {
    let state = iterations_state();
    let absent =
        augment_message_delta_usage(&state, object(r#"{"type": "message_delta", "usage": {}}"#))
            .expect("augmented");
    assert_eq!(
        absent.get("usage"),
        Some(&json_object(
            r#"{"iterations": [{"type": "compaction", "input_tokens": 12, "output_tokens": 3}]}"#
        ))
    );
    let nulls = augment_message_delta_usage(
        &state,
        object(
            r#"{"type": "message_delta", "usage": {"input_tokens": null, "output_tokens": null}}"#,
        ),
    )
    .expect("augmented");
    assert_eq!(
        nulls.get("usage"),
        Some(&json_object(
            r#"{"input_tokens": null, "output_tokens": null,
                "iterations": [{"type": "compaction", "input_tokens": 12, "output_tokens": 3}]}"#
        ))
    );
}

#[rstest]
#[case::empty_iterations(
    r#"{"type": "message_delta", "usage": {"input_tokens": 5, "output_tokens": 2, "iterations": []}}"#
)]
#[case::null_iterations(
    r#"{"type": "message_delta", "usage": {"input_tokens": 5, "output_tokens": 2, "iterations": null}}"#
)]
fn existing_iterations_block_the_message_iteration(#[case] event: &str) {
    let augmented =
        augment_message_delta_usage(&iterations_state(), object(event)).expect("augmented");
    assert_eq!(augmented, object(event));
}

#[rstest]
#[case::usage_absent(r#"{"type": "message_delta"}"#)]
#[case::usage_null(r#"{"type": "message_delta", "usage": null}"#)]
#[case::usage_a_list(r#"{"type": "message_delta", "usage": []}"#)]
fn non_object_usage_blocks_iterations(#[case] event: &str) {
    let augmented =
        augment_message_delta_usage(&iterations_state(), object(event)).expect("augmented");
    assert_eq!(augmented, object(event));
}

#[test]
fn iterations_absent_still_attaches_context_edits() {
    let state = state_with(StreamSetup {
        applied_edits: vec![applied_edit()],
        ..setup()
    });
    let augmented = augment_message_delta_usage(
        &state,
        object(r#"{"type": "message_delta", "usage": {"input_tokens": 5, "output_tokens": 2}}"#),
    )
    .expect("augmented");
    assert_eq!(
        augmented,
        object(
            r#"{"type": "message_delta", "usage": {"input_tokens": 5, "output_tokens": 2},
                "context_management": {"applied_edits": [
                    {"type": "clear_tool_uses_20250919", "cleared_tool_uses": 1}
                ]}}"#
        )
    );
    let retained = augment_message_delta_usage(
        &state,
        object(r#"{"type": "message_delta", "context_management": null}"#),
    )
    .expect("augmented");
    assert_eq!(retained.get("context_management"), Some(&Json::Null));
}

#[test]
fn compaction_three_step_lifecycle() {
    let mut state = compaction_state(Some("S"));
    let start = next_compaction_event(&mut state).expect("start");
    assert_eq!(
        start,
        object(
            r#"{"type": "content_block_start", "index": 0,
                "content_block": {"type": "compaction", "content": ""}}"#
        )
    );
    let delta = next_compaction_event(&mut state).expect("delta");
    assert_eq!(
        delta,
        object(
            r#"{"type": "content_block_delta", "index": 0,
                "delta": {"type": "compaction_delta", "content": "S"}}"#
        )
    );
    let stop = next_compaction_event(&mut state).expect("stop");
    assert_eq!(
        stop,
        object(r#"{"type": "content_block_stop", "index": 0}"#)
    );
    assert_eq!(state.current_content_block_index, 1);
    assert!(state.sent_compaction_block);
    assert!(!state.block_progress.sent_content_block_finish);
    assert!(next_compaction_event(&mut state).is_none());
}

#[test]
fn compaction_flags_follow_the_returned_event() {
    let mut state = compaction_state(Some("S"));
    assert!(next_compaction_event(&mut state).is_some());
    assert!(state.compaction_progress.sent_compaction_block_start);
    assert!(!state.compaction_progress.sent_compaction_block_delta);
    assert!(!state.sent_compaction_block);
    assert_eq!(state.current_content_block_index, 0);
    assert!(next_compaction_event(&mut state).is_some());
    assert!(state.compaction_progress.sent_compaction_block_delta);
    assert!(!state.sent_compaction_block);
    assert_eq!(state.current_content_block_index, 0);
    assert!(next_compaction_event(&mut state).is_some());
    assert_eq!(state.current_content_block_index, 1);
}

#[rstest]
#[case::block_without_content(None)]
#[case::block_with_empty_content(Some(""))]
fn compaction_delta_without_summary_is_empty(#[case] content: Option<&str>) {
    let mut state = compaction_state(content);
    assert!(next_compaction_event(&mut state).is_some());
    let delta = next_compaction_event(&mut state).expect("delta");
    assert_eq!(
        delta,
        object(
            r#"{"type": "content_block_delta", "index": 0,
                "delta": {"type": "compaction_delta", "content": ""}}"#
        )
    );
}

#[test]
fn absent_compaction_block_emits_nothing() {
    let mut state = state();
    assert!(next_compaction_event(&mut state).is_none());
    assert!(!state.compaction_progress.sent_compaction_block_start);
    assert!(!state.compaction_progress.sent_compaction_block_delta);
    assert!(!state.sent_compaction_block);
    assert_eq!(state.current_content_block_index, 0);
}

#[rstest]
#[case::text_present(
    r#"{"type": "content_block_delta", "delta": {"type": "text_delta", "text": "x"}}"#,
    true
)]
#[case::text_empty(
    r#"{"type": "content_block_delta", "delta": {"type": "text_delta", "text": ""}}"#,
    false
)]
#[case::text_absent(
    r#"{"type": "content_block_delta", "delta": {"type": "text_delta"}}"#,
    false
)]
#[case::input_json_present(
    r#"{"type": "content_block_delta", "delta": {"type": "input_json_delta", "partial_json": "{}"}}"#,
    true
)]
#[case::thinking_present(
    r#"{"type": "content_block_delta", "delta": {"type": "thinking_delta", "thinking": "t"}}"#,
    true
)]
#[case::signature_present(
    r#"{"type": "content_block_delta", "delta": {"type": "signature_delta", "signature": "s"}}"#,
    true
)]
#[case::text_delta_reads_only_its_own_payload(
    r#"{"type": "content_block_delta", "delta": {"type": "text_delta", "partial_json": "x"}}"#,
    false
)]
#[case::compaction_delta_is_not_a_translated_type(
    r#"{"type": "content_block_delta", "delta": {"type": "compaction_delta", "content": "x"}}"#,
    false
)]
#[case::unknown_delta_type(
    r#"{"type": "content_block_delta", "delta": {"type": "unknown_delta", "text": "x"}}"#,
    false
)]
#[case::delta_not_an_object(r#"{"type": "content_block_delta", "delta": null}"#, false)]
#[case::outer_type_is_message_delta(
    r#"{"type": "message_delta", "delta": {"type": "text_delta", "text": "x"}}"#,
    false
)]
#[case::outer_type_absent(r#"{"delta": {"type": "text_delta", "text": "x"}}"#, false)]
fn nonempty_delta_predicates(#[case] source: &str, #[case] expected: bool) {
    assert_eq!(delta_has_content(&object(source)), expected, "{source}");
}

#[rstest]
#[case::finish_reason_present(r#"{"finish_reason": "stop", "delta": {}}"#, false)]
#[case::finish_reason_empty_string(r#"{"finish_reason": "", "delta": {}}"#, false)]
#[case::tool_calls_present(
    r#"{"delta": {"tool_calls": [{"id": "t", "function": {"name": "short", "arguments": "{}"}}]}}"#,
    false
)]
#[case::content_present(r#"{"delta": {"content": "x"}}"#, false)]
#[case::content_empty_string(r#"{"delta": {"content": ""}}"#, true)]
#[case::refusal_present(r#"{"delta": {"refusal": "no"}}"#, false)]
#[case::refusal_empty_string(r#"{"delta": {"refusal": ""}}"#, true)]
#[case::refusal_in_provider_specific_fields(
    r#"{"delta": {"refusal": null, "provider_specific_fields": {"refusal": "No"}}}"#,
    false
)]
#[case::provider_specific_fields_refusal_empty_string(
    r#"{"delta": {"refusal": null, "provider_specific_fields": {"refusal": ""}}}"#,
    true
)]
#[case::reasoning_content_present(r#"{"delta": {"reasoning_content": "r"}}"#, false)]
#[case::empty_delta(r#"{"delta": {}}"#, true)]
#[case::empty_thinking_blocks(r#"{"delta": {"thinking_blocks": []}}"#, true)]
#[case::absent_thinking_blocks(r#"{"delta": {"thinking_blocks": null}}"#, true)]
fn blank_delta_predicates(#[case] source: &str, #[case] expected: bool) {
    let choices = vec![choice(source)];
    let blank = is_blank_delta(&choices).expect("one choice");
    assert_eq!(blank, expected, "{source}");
}

#[test]
fn thought_blocks_decide_blankness() {
    let unsigned = vec![choice(
        r#"{"delta": {"thinking_blocks": [{"type": "thinking", "thinking": "", "signature": ""}]}}"#,
    )];
    assert!(is_blank_delta(&unsigned).expect("one choice"));
    let signed = vec![choice(
        r#"{"delta": {"thinking_blocks": [{"type": "thinking", "thinking": "", "signature": "sig"}]}}"#,
    )];
    assert!(!is_blank_delta(&signed).expect("one choice"));
    let filled = vec![choice(
        r#"{"delta": {"thinking_blocks": [{"type": "thinking", "thinking": "t", "signature": ""}]}}"#,
    )];
    assert!(!is_blank_delta(&filled).expect("one choice"));
}

#[test]
fn blankness_reads_only_the_first_choice() {
    let choices = vec![
        choice(r#"{"delta": {}}"#),
        choice(r#"{"delta": {"content": "x"}}"#),
    ];
    assert!(is_blank_delta(&choices).expect("two choices"));
}

#[test]
fn blankness_without_choices_is_an_error() {
    assert!(matches!(
        is_blank_delta(&[]),
        Err(StreamStateError::Content { .. })
    ));
}

#[test]
fn content_boundary_and_tool_restore() {
    let mut state = state_with(StreamSetup {
        tool_name_mapping: IndexMap::from([("short".to_owned(), "long_tool_name".to_owned())]),
        ..setup()
    });
    assert!(!start(&mut state, r#"{"delta": {"content": "hi"}}"#));
    assert_eq!(state.current_content_block_type, BlockKind::Text);
    assert_eq!(
        state.current_content_block_start,
        json_object(r#"{"type": "text", "text": ""}"#)
    );
    assert!(start(
        &mut state,
        r#"{"delta": {"reasoning_content": "r"}}"#
    ));
    assert_eq!(state.current_content_block_type, BlockKind::Thinking);
    assert_eq!(
        state.current_content_block_start,
        json_object(r#"{"type": "thinking", "thinking": "", "signature": ""}"#)
    );
    let named = r#"{"delta": {"tool_calls": [{"id": "tool_1", "function": {"name": "short", "arguments": "{}"}}]}}"#;
    assert!(start(&mut state, named));
    assert_eq!(state.current_content_block_type, BlockKind::ToolUse);
    assert_eq!(
        state.current_content_block_start,
        json_object(
            r#"{"type": "tool_use", "id": "tool_1", "name": "long_tool_name", "input": {}}"#
        )
    );
    let later = r#"{"delta": {"tool_calls": [{"id": "tool_2", "function": {"name": "later", "arguments": "{}"}}]}}"#;
    assert!(start(&mut state, later));
    assert_eq!(
        state.current_content_block_start,
        json_object(r#"{"type": "tool_use", "id": "tool_2", "name": "later", "input": {}}"#)
    );
}

#[test]
fn nameless_tool_arguments_keep_the_block() {
    let mut state = state();
    let named = r#"{"delta": {"tool_calls": [{"id": "tool_1", "function": {"name": "short", "arguments": "{}"}}]}}"#;
    assert!(start(&mut state, named));
    let before = state.current_content_block_start.clone();
    let continuation =
        r#"{"delta": {"tool_calls": [{"id": "tool_1", "function": {"arguments": "{\"a\": 1}"}}]}}"#;
    assert!(!start(&mut state, continuation));
    assert_eq!(state.current_content_block_start, before);
}

#[test]
fn text_block_start_is_not_replaced() {
    let mut state = state();
    state.current_content_block_start = Json::Text("sentinel".to_owned());
    assert!(!start(&mut state, r#"{"delta": {"content": "hi"}}"#));
    assert_eq!(
        state.current_content_block_start,
        Json::Text("sentinel".to_owned())
    );
}

#[test]
fn finish_chunk_returns_before_refusal_collection() {
    let mut state = state();
    assert!(!start(
        &mut state,
        r#"{"finish_reason": "stop", "delta": {"content": "bye", "refusal": "ignored"}}"#
    ));
    assert!(state.refusal_text.is_empty());
}

#[test]
fn refusal_fragments_accumulate_in_order() {
    let mut state = state();
    assert!(!start(&mut state, r#"{"delta": {"refusal": "No"}}"#));
    assert!(!start(&mut state, r#"{"delta": {"refusal": " thanks"}}"#));
    assert_eq!(state.refusal_text, "No thanks");
}

#[test]
fn refusal_in_provider_specific_fields_becomes_the_explanation() {
    let mut state = state();
    assert!(!start(
        &mut state,
        r#"{"delta": {"refusal": null, "provider_specific_fields": {"refusal": "No"}}}"#
    ));
    assert_eq!(state.refusal_text, "No");
    let event = object(r#"{"type": "message_delta", "delta": {"stop_reason": "end_turn"}}"#);
    let processed = with_refusal_stop_details(&state, event).expect("processed");
    assert_eq!(
        processed.get("delta").expect("delta"),
        &json_object(
            r#"{"stop_reason": "refusal",
                "stop_details": {"type": "refusal", "category": null, "explanation": "No"}}"#
        )
    );
}

#[test]
fn start_block_without_choices_is_an_error() {
    let mut state = state();
    assert!(matches!(
        should_start_new_content_block(&mut state, &[], &FixedIds),
        Err(StreamStateError::Content { .. })
    ));
}

#[test]
fn refusal_stop_precedence() {
    let mut state = state();
    assert!(!start(&mut state, r#"{"delta": {"refusal": "No"}}"#));
    assert!(!start(&mut state, r#"{"delta": {"refusal": " thanks"}}"#));
    let event = object(
        r#"{"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": "x"}, "usage": {"input_tokens": 1}}"#,
    );
    let processed = with_refusal_stop_details(&state, event).expect("processed");
    assert_eq!(
        processed,
        object(
            r#"{"type": "message_delta",
                "delta": {"stop_reason": "refusal", "stop_sequence": "x",
                          "stop_details": {"type": "refusal", "category": null,
                                           "explanation": "No thanks"}},
                "usage": {"input_tokens": 1}}"#
        )
    );
}

#[test]
fn max_tokens_is_not_overridden() {
    let mut state = state();
    assert!(!start(&mut state, r#"{"delta": {"refusal": "No"}}"#));
    let event = object(r#"{"type": "message_delta", "delta": {"stop_reason": "max_tokens"}}"#);
    let processed = with_refusal_stop_details(&state, event.clone()).expect("processed");
    assert_eq!(processed, event);
}

#[test]
fn nonterminal_or_empty_refusal_is_unchanged() {
    let empty = state();
    let terminal = object(r#"{"type": "message_delta", "delta": {"stop_reason": "end_turn"}}"#);
    let terminal_result = with_refusal_stop_details(&empty, terminal.clone()).expect("processed");
    assert_eq!(terminal_result, terminal);
    let mut state = state();
    assert!(!start(&mut state, r#"{"delta": {"refusal": "No"}}"#));
    let content =
        object(r#"{"type": "content_block_delta", "delta": {"type": "text_delta", "text": "x"}}"#);
    let content_result = with_refusal_stop_details(&state, content.clone()).expect("processed");
    assert_eq!(content_result, content);
}

#[rstest]
#[case::delta_absent(r#"{"type": "message_delta"}"#, "delta")]
#[case::delta_null(r#"{"type": "message_delta", "delta": null}"#, "delta")]
#[case::type_absent(r#"{"delta": {"stop_reason": "end_turn"}}"#, "type")]
fn malformed_refusal_events_are_errors(#[case] event: &str, #[case] field: &str) {
    let mut state = state();
    assert!(!start(&mut state, r#"{"delta": {"refusal": "No"}}"#));
    assert!(
        matches!(
            with_refusal_stop_details(&state, object(event)),
            Err(StreamStateError::MalformedEvent { field: missing }) if missing == field
        ),
        "{event}"
    );
}

#[test]
fn existing_stop_details_are_replaced() {
    let mut state = state();
    assert!(!start(&mut state, r#"{"delta": {"refusal": "No"}}"#));
    let event = object(
        r#"{"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_details": {"type": "other"}}}"#,
    );
    let processed = with_refusal_stop_details(&state, event).expect("processed");
    assert_eq!(
        processed.get("delta").expect("delta"),
        &json_object(
            r#"{"stop_reason": "refusal",
                "stop_details": {"type": "refusal", "category": null, "explanation": "No"}}"#
        )
    );
}
