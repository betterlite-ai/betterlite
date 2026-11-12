#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::error::StreamToolsError;
use super::final_tools::queue_final_tool_completion;
use super::items::{has_signed_thinking_block, index_of_output_item_type, output_items_with_id};
use super::reasoning::reasoning_ended;
use super::state::{
    ChatStreamState, FIRST_TOOL_OUTPUT_INDEX, StreamSetup, assign_tool_output_index, next_sequence,
};
use super::tool_calls::{normalize_tool_call_index, queue_tool_delta_events, tool_call_id};
use super::web_search::reserve_web_search_indexes;
use crate::api::api_boundaries::ApiLimits;

mod fixtures;
mod hosted_search;
use fixtures::{deltas, events, item_id, json, output_item, state_with, state_with_metadata};

pub(super) fn state_without_tools() -> ChatStreamState<String> {
    state_with(json("[]"))
}

#[test]
fn stream_state_defaults_start_unfinished() {
    let state = state_without_tools();
    assert!(!state.progress.finished);
    assert!(!state.progress.sent_response_created);
    assert!(!state.progress.sent_in_progress);
    assert!(!state.progress.sent_output_item_added);
    assert!(!state.progress.sent_message_item_added);
    assert!(!state.progress.sent_content_part_added);
    assert!(!state.progress.sent_output_text_done);
    assert!(!state.progress.sent_content_part_done);
    assert!(!state.progress.sent_output_item_done);
}

#[test]
fn stream_state_defaults_start_unprimed() {
    let state = state_without_tools();
    assert!(!state.progress.sent_annotation_events);
    assert!(!state.progress.upstream_exhausted);
    assert!(!state.progress.response_id_primed);
    assert!(!state.progress.final_tool_events_queued);
    assert!(!state.progress.reasoning_active);
    assert!(!state.progress.reasoning_done_emitted);
    assert!(!state.progress.sent_reasoning_text_done);
    assert!(!state.progress.sent_reasoning_part_done);
}

#[test]
fn stream_state_defaults_start_at_zero() {
    let state = state_without_tools();
    assert_eq!(state.progress.sequence_number, 0);
    assert_eq!(state.progress.message_output_index, 0);
    assert_eq!(
        state.progress.next_tool_output_index,
        FIRST_TOOL_OUTPUT_INDEX
    );
}

#[test]
fn stream_state_defaults_start_with_empty_queues() {
    let state = state_without_tools();
    assert!(state.progress.pending_tool_events.is_empty());
    assert!(state.progress.pending_response_events.is_empty());
    assert!(state.progress.reasoning_content_parts.is_empty());
    assert!(state.progress.provider_specific_fields.is_empty());
    assert!(state.progress.chunk_snapshots.is_empty());
}

#[test]
fn stream_state_defaults_start_with_empty_maps() {
    let state = state_without_tools();
    assert!(state.progress.tool_output_index_by_call_id.is_empty());
    assert!(state.progress.tool_args_by_call_id.is_empty());
    assert!(state.progress.tool_item_id_by_call_id.is_empty());
    assert!(state.progress.tool_call_id_by_index.is_empty());
    assert!(state.progress.ambiguous_tool_call_indexes.is_empty());
    assert!(state.progress.web_search_calls.is_empty());
    assert!(state.progress.queued_web_search_call_ids.is_empty());
}

#[test]
fn stream_state_defaults_hold_no_identity() {
    let state = state_without_tools();
    assert!(state.progress.cached_item_id.is_none());
    assert!(state.progress.cached_response_id.is_none());
    assert!(state.progress.cached_reasoning_item_id.is_none());
    assert!(state.progress.buffered_chunk.is_none());
    assert!(state.progress.litellm_model_response.is_none());
    assert!(state.progress.completed_response.is_none());
    assert!(state.progress.reasoning_item_id.is_none());
}

#[test]
fn stream_state_defaults_copy_the_setup() {
    let state = state_without_tools();
    assert_eq!(state.progress.final_text, "");
    assert_eq!(state.progress.reasoning_summary_text, "");
    assert_eq!(state.model, "gpt-4o-mini");
    assert_eq!(state.logging_context, "wrapper-context");
    assert_eq!(state.request_input, json("\"hi\""));
}

#[test]
fn stream_state_defaults_replace_null_metadata() {
    let state = state_with_metadata(json("[]"), None);
    assert!(state.litellm_metadata.is_empty());
    let mut metadata = IndexMap::new();
    metadata.insert(String::from("owner"), Json::Text(String::from("team")));
    let with_metadata = state_with_metadata(json("[]"), Some(metadata));
    assert_eq!(with_metadata.litellm_metadata.len(), 1);
}

#[test]
fn stream_state_defaults_read_custom_and_namespace_tools() {
    let tools = json(
        r#"[{"type":"custom","name":"patch"},{"type":"namespace","name":"ns","tools":[{"type":"function","name":"f"}]}]"#,
    );
    let state = state_with(tools);
    assert!(state.custom_tool_names.contains("patch"));
    assert_eq!(state.namespace_tool_names.len(), 2);
    let bare = state
        .namespace_tool_names
        .get("f")
        .expect("the bare name maps back");
    assert_eq!(bare.namespace, "ns");
    assert_eq!(bare.name, "f");
    let qualified = state
        .namespace_tool_names
        .get("ns__f")
        .expect("the qualified name maps back");
    assert_eq!(qualified.name, "f");
}

#[rstest]
#[case::missing("{}")]
#[case::null(r#"{"tools":null}"#)]
fn stream_state_defaults_accept_missing_tools(#[case] text: &str) {
    let request = json(text);
    let state = ChatStreamState::<String>::new(StreamSetup {
        model: "m",
        logging_context: String::new(),
        request_input: &Json::Null,
        responses_api_request: &request,
        custom_llm_provider: None,
        litellm_metadata: None,
        limits: ApiLimits::default(),
    })
    .expect("the state builds");
    assert!(state.custom_tool_names.is_empty());
    assert!(state.namespace_tool_names.is_empty());
}

#[test]
fn stream_state_defaults_reject_tools_that_are_not_a_list() {
    let mut fields = IndexMap::new();
    fields.insert(
        String::from("tools"),
        Json::Text(String::from("web_search")),
    );
    let request = Json::Object(fields);
    let result = ChatStreamState::<String>::new(StreamSetup {
        model: "m",
        logging_context: String::new(),
        request_input: &Json::Null,
        responses_api_request: &request,
        custom_llm_provider: None,
        litellm_metadata: None,
        limits: ApiLimits::default(),
    });
    assert!(matches!(result, Err(StreamToolsError::ToolsNotList)));
}

#[test]
fn tool_output_indices_are_assigned_once_per_call_id() {
    let mut state = state_without_tools();
    assert_eq!(assign_tool_output_index(&mut state, "c").unwrap(), 1);
    assert_eq!(assign_tool_output_index(&mut state, "c").unwrap(), 1);
    assert_eq!(assign_tool_output_index(&mut state, "d").unwrap(), 2);
    assert_eq!(state.progress.next_tool_output_index, 3);
}

#[test]
fn tool_output_indices_past_the_integer_range_are_errors() {
    let mut state = state_without_tools();
    state.progress.next_tool_output_index = i64::MAX;
    assert!(matches!(
        assign_tool_output_index(&mut state, "c"),
        Err(StreamToolsError::IndexOverflow)
    ));
    state.progress.sequence_number = i64::MAX;
    assert!(matches!(
        next_sequence(&mut state),
        Err(StreamToolsError::SequenceOverflow)
    ));
}

#[test]
fn item_id_replacement_copies_only_the_first_typed_matching_item() {
    let items = vec![
        output_item(r#"{"type":"message","id":"dict_1"}"#),
        output_item(
            r#"{"type":"message","id":"msg_1","content":[],"role":"assistant","status":"in_progress"}"#,
        ),
        output_item(
            r#"{"type":"message","id":"msg_2","content":[],"role":"assistant","status":"in_progress"}"#,
        ),
    ];
    assert_eq!(index_of_output_item_type(&items, "message"), Some(1));
    let replaced = output_items_with_id(&items, "message", Some("msg_new"));
    assert_eq!(item_id(&replaced[0]), json("\"dict_1\""));
    assert_eq!(item_id(&replaced[1]), json("\"msg_new\""));
    assert_eq!(item_id(&replaced[2]), json("\"msg_2\""));
}

#[test]
fn item_id_replacement_leaves_items_alone_for_a_null_id_or_another_type() {
    let items = vec![
        output_item(
            r#"{"type":"message","id":"msg_1","content":[],"role":"assistant","status":"in_progress"}"#,
        ),
        output_item(r#"{"type":"reasoning","id":"rs_1","summary":[]}"#),
    ];
    let untouched = output_items_with_id(&items, "message", None);
    assert_eq!(item_id(&untouched[0]), json("\"msg_1\""));
    assert_eq!(item_id(&untouched[1]), json("\"rs_1\""));
    let other = output_items_with_id(&items, "function_call", Some("call_new"));
    assert_eq!(item_id(&other[0]), json("\"msg_1\""));
    assert_eq!(item_id(&other[1]), json("\"rs_1\""));
}

#[test]
fn item_id_replacement_covers_sdk_items_and_ignores_untyped_dicts() {
    let items = vec![
        output_item(r#"{"type":"reasoning","id":"dict_1"}"#),
        output_item(r#"{"type":"reasoning","id":"rs_1","summary":[]}"#),
    ];
    assert_eq!(index_of_output_item_type(&items, "reasoning"), Some(1));
    let replaced = output_items_with_id(&items, "reasoning", Some("rs_new"));
    assert_eq!(item_id(&replaced[0]), json("\"dict_1\""));
    assert_eq!(item_id(&replaced[1]), json("\"rs_new\""));
}

#[test]
fn item_id_replacement_leaves_other_types_alone() {
    let items = vec![
        output_item(
            r#"{"type":"message","id":"msg_1","content":[],"role":"assistant","status":"in_progress"}"#,
        ),
        output_item(
            r#"{"type":"function_call","id":"fc_1","call_id":"c","name":"f","arguments":"","status":"in_progress"}"#,
        ),
    ];
    let replaced = output_items_with_id(&items, "reasoning", Some("rs_new"));
    assert_eq!(item_id(&replaced[0]), json("\"msg_1\""));
    assert_eq!(item_id(&replaced[1]), json("\"fc_1\""));
    assert_eq!(index_of_output_item_type(&items, "reasoning"), None);
}

#[rstest]
#[case::signature(r#"{"thinking_blocks":[{"signature":"s"}]}"#, true)]
#[case::data(r#"{"thinking_blocks":[{"data":1}]}"#, true)]
#[case::both_falsy(r#"{"thinking_blocks":[{"signature":"","data":0}]}"#, false)]
#[case::second_block(r#"{"thinking_blocks":[{},{"signature":"s"}]}"#, true)]
#[case::text_blocks(r#"{"thinking_blocks":"signature"}"#, false)]
#[case::absent("{}", false)]
#[case::empty(r#"{"thinking_blocks":[]}"#, false)]
fn signed_thinking_blocks_keep_their_python_truthiness(
    #[case] delta: &str,
    #[case] expected: bool,
) {
    assert_eq!(has_signed_thinking_block(&json(delta)), expected);
}

#[rstest]
#[case::no_choices("{}")]
#[case::empty_choices(r#"{"choices":[]}"#)]
fn reasoning_end_ignores_a_chunk_without_choices(#[case] chunk: &str) {
    assert!(!reasoning_ended(&json(chunk)));
}

#[rstest]
#[case::reasoning(r#"{"choices":[{"delta":{"reasoning_content":"r"}}]}"#, false)]
#[case::signed_blocks(
    r#"{"choices":[{"delta":{"thinking_blocks":[{"signature":"s"}]}}]}"#,
    false
)]
#[case::text_blocks(r#"{"choices":[{"delta":{"thinking_blocks":"x"}}]}"#, false)]
#[case::text(r#"{"choices":[{"delta":{"content":"hi"}}]}"#, true)]
#[case::empty_text(r#"{"choices":[{"delta":{"content":""}}]}"#, false)]
#[case::function_call(r#"{"choices":[{"delta":{"function_call":{"name":"f"}}}]}"#, true)]
#[case::tool_calls(r#"{"choices":[{"delta":{"tool_calls":[{"id":"c"}]}}]}"#, true)]
#[case::empty_tool_calls(r#"{"choices":[{"delta":{"tool_calls":[]}}]}"#, false)]
#[case::empty_finish(r#"{"choices":[{"delta":{},"finish_reason":""}]}"#, true)]
#[case::null_finish(r#"{"choices":[{"delta":{},"finish_reason":null}]}"#, false)]
#[case::first_choice_only(r#"{"choices":[{"delta":{}},{"delta":{"content":"hi"}}]}"#, false)]
fn reasoning_end_reads_only_the_first_choice(#[case] chunk: &str, #[case] expected: bool) {
    assert_eq!(reasoning_ended(&json(chunk)), expected);
}

#[rstest]
#[case::string(r#"{"index":" +2 "}"#, Some(2))]
#[case::true_value(r#"{"index":true}"#, Some(1))]
#[case::false_value(r#"{"index":false}"#, Some(0))]
#[case::float(r#"{"index":2.9}"#, Some(2))]
#[case::negative_float(r#"{"index":-2.9}"#, Some(-2))]
#[case::negative(r#"{"index":-3}"#, Some(-3))]
#[case::integer(r#"{"index":7}"#, Some(7))]
#[case::null_value(r#"{"index":null}"#, None)]
#[case::text(r#"{"index":"2.9"}"#, None)]
#[case::empty_text(r#"{"index":""}"#, None)]
#[case::missing_value("{}", None)]
#[case::list(r#"{"index":[1]}"#, None)]
#[case::object(r#"{"index":{"a":1}}"#, None)]
fn call_index_correlation_normalizes_like_int(#[case] call: &str, #[case] expected: Option<i64>) {
    assert_eq!(normalize_tool_call_index(&json(call)), expected);
}

#[test]
fn call_index_correlation_remembers_explicit_ids_and_drops_ambiguous_indexes() {
    let mut state = state_without_tools();
    assert_eq!(
        tool_call_id(&mut state, &json(r#"{"index":0,"id":"a"}"#)),
        "a"
    );
    assert_eq!(tool_call_id(&mut state, &json(r#"{"index":0}"#)), "a");
    assert_eq!(
        tool_call_id(&mut state, &json(r#"{"index":0,"id":"b"}"#)),
        "b"
    );
    assert_eq!(tool_call_id(&mut state, &json(r#"{"index":0}"#)), "");
    assert_eq!(
        tool_call_id(&mut state, &json(r#"{"index":0,"id":"c"}"#)),
        "c"
    );
    assert_eq!(tool_call_id(&mut state, &json(r#"{"index":0}"#)), "");
    assert_eq!(
        tool_call_id(&mut state, &json(r#"{"index":4,"id":"d"}"#)),
        "d"
    );
    assert_eq!(tool_call_id(&mut state, &json(r#"{"index":4}"#)), "d");
}

#[test]
fn call_index_correlation_stringifies_ids_and_keeps_indexless_ones_out() {
    let mut state = state_without_tools();
    assert_eq!(
        tool_call_id(&mut state, &json(r#"{"index":1,"id":42}"#)),
        "42"
    );
    assert_eq!(
        tool_call_id(&mut state, &json(r#"{"id":"no-index"}"#)),
        "no-index"
    );
    assert_eq!(tool_call_id(&mut state, &json(r#"{"index":1}"#)), "42");
    assert!(!state.progress.tool_call_id_by_index.contains_key(&2));
    assert_eq!(tool_call_id(&mut state, &json("{}")), "");
    assert_eq!(tool_call_id(&mut state, &json(r#"{"index":0}"#)), "");
    assert_eq!(tool_call_id(&mut state, &json(r#"{"id":""}"#)), "");
}

#[test]
fn tool_delta_queue_emits_added_then_ten_character_deltas() {
    let mut state = state_without_tools();
    queue_tool_delta_events(
        &mut state,
        &json(r#"[{"id":"c","index":0,"function":{"name":"f","arguments":"12345678901"}}]"#),
    )
    .unwrap();
    let queued = events(&state);
    assert_eq!(queued.len(), 3);
    assert_eq!(
        queued[0],
        json(
            r#"{"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","id":"fc_c","call_id":"c","name":"f","status":"in_progress","arguments":""},"sequence_number":1}"#
        )
    );
    assert_eq!(
        queued[1],
        json(
            r#"{"type":"response.function_call_arguments.delta","item_id":"fc_c","output_index":1,"delta":"1234567890","sequence_number":2}"#
        )
    );
    assert_eq!(
        queued[2],
        json(
            r#"{"type":"response.function_call_arguments.delta","item_id":"fc_c","output_index":1,"delta":"1","sequence_number":3}"#
        )
    );
    assert_eq!(
        state
            .progress
            .tool_args_by_call_id
            .get("c")
            .map(String::as_str),
        Some("12345678901")
    );
}

#[test]
fn tool_delta_queue_appends_later_deltas_without_another_item() {
    let mut state = state_without_tools();
    queue_tool_delta_events(&mut state, &deltas("12")).unwrap();
    queue_tool_delta_events(&mut state, &deltas("AB")).unwrap();
    let queued = events(&state);
    assert_eq!(queued.len(), 3);
    assert_eq!(
        queued[2],
        json(
            r#"{"type":"response.function_call_arguments.delta","item_id":"fc_c","output_index":1,"delta":"AB","sequence_number":3}"#
        )
    );
    assert_eq!(
        state
            .progress
            .tool_args_by_call_id
            .get("c")
            .map(String::as_str),
        Some("12AB")
    );
}

#[rstest]
#[case::null_value("null")]
#[case::object(r#"{"id":"c"}"#)]
#[case::text("\"c\"")]
fn tool_delta_queue_ignores_a_tool_calls_value_that_is_not_a_list(#[case] tool_calls: &str) {
    let mut state = state_without_tools();
    queue_tool_delta_events(&mut state, &json(tool_calls)).unwrap();
    assert!(events(&state).is_empty());
    assert_eq!(state.progress.sequence_number, 0);
}

#[test]
fn tool_delta_queue_ignores_ids_it_cannot_resolve() {
    let mut state = state_with(json(r#"[{"type":"function","name":"f"}]"#));
    queue_tool_delta_events(
        &mut state,
        &json(r#"[{"index":5,"function":{"name":"f","arguments":"a"}}]"#),
    )
    .unwrap();
    assert!(events(&state).is_empty());
    assert!(state.progress.tool_args_by_call_id.is_empty());
}

#[test]
fn tool_delta_queue_ignores_web_search_ids() {
    let mut state = state_with(json(r#"[{"type":"function","name":"f"}]"#));
    reserve_web_search_indexes(
        &mut state,
        &json(
            r#"{"web_search_calls":{"k":{"id":"ws_call_x","status":"completed","action":{"type":"search"}}}}"#,
        ),
    )
    .unwrap();
    queue_tool_delta_events(
        &mut state,
        &json(r#"[{"id":"call_x","index":1,"function":{"name":"f","arguments":"a"}}]"#),
    )
    .unwrap();
    assert!(events(&state).is_empty());
    assert!(state.progress.tool_args_by_call_id.is_empty());
}

#[test]
fn tool_delta_queue_normalizes_missing_names_and_arguments() {
    let mut state = state_without_tools();
    queue_tool_delta_events(&mut state, &json(r#"[{"id":"c","index":0,"function":{}}]"#)).unwrap();
    let queued = events(&state);
    assert_eq!(queued.len(), 1);
    assert_eq!(
        queued[0],
        json(
            r#"{"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","id":"fc_c","call_id":"c","name":"","status":"in_progress","arguments":""},"sequence_number":1}"#
        )
    );
}

#[test]
fn tool_delta_queue_serializes_nonstring_arguments() {
    let mut state = state_without_tools();
    queue_tool_delta_events(
        &mut state,
        &json(r#"[{"id":"d","index":0,"function":{"name":"f","arguments":{"a":1}}}]"#),
    )
    .unwrap();
    let queued = events(&state);
    assert_eq!(
        queued[1],
        json(
            r#"{"type":"response.function_call_arguments.delta","item_id":"fc_d","output_index":1,"delta":"{\"a\": 1}","sequence_number":2}"#
        )
    );
    assert_eq!(
        state
            .progress
            .tool_args_by_call_id
            .get("d")
            .map(String::as_str),
        Some("{\"a\": 1}")
    );
}

#[test]
fn tool_delta_queue_restores_namespaces_only_for_function_calls() {
    let tools =
        json(r#"[{"type":"namespace","name":"ns","tools":[{"type":"function","name":"f"}]}]"#);
    let mut state = state_with(tools);
    queue_tool_delta_events(
        &mut state,
        &json(r#"[{"id":"c","index":0,"function":{"name":"ns__f","arguments":"ab"}}]"#),
    )
    .unwrap();
    let queued = events(&state);
    assert_eq!(
        queued[0],
        json(
            r#"{"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","id":"fc_c","call_id":"c","name":"f","status":"in_progress","arguments":"","namespace":"ns"},"sequence_number":1}"#
        )
    );
}

#[test]
fn tool_delta_queue_keeps_the_custom_item_shape() {
    let mut state = state_with(json(r#"[{"type":"custom","name":"patch"}]"#));
    queue_tool_delta_events(
        &mut state,
        &json(r#"[{"id":"c","index":0,"function":{"name":"patch","arguments":"{\"content\":\"x\"}"}}]"#),
    )
    .unwrap();
    let queued = events(&state);
    assert_eq!(
        queued[0],
        json(
            r#"{"type":"response.output_item.added","output_index":1,"item":{"type":"custom_tool_call","id":"ctc_c","call_id":"c","name":"patch","status":"in_progress","input":""},"sequence_number":1}"#
        )
    );
    assert_eq!(
        queued[1],
        json(
            r#"{"type":"response.function_call_arguments.delta","item_id":"ctc_c","output_index":1,"delta":"{\"content\"","sequence_number":2}"#
        )
    );
}

#[test]
fn tool_delta_queue_keeps_ids_that_already_carry_their_prefix() {
    let mut state = state_without_tools();
    queue_tool_delta_events(
        &mut state,
        &json(r#"[{"id":"fc_ready","index":0,"function":{"name":"f","arguments":""}}]"#),
    )
    .unwrap();
    let queued = events(&state);
    assert_eq!(
        queued[0],
        json(
            r#"{"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","id":"fc_ready","call_id":"fc_ready","name":"f","status":"in_progress","arguments":""},"sequence_number":1}"#
        )
    );
}

#[test]
fn tool_delta_queue_splits_by_characters_not_bytes() {
    let mut state = state_without_tools();
    let arguments = "\u{e9}".repeat(11);
    queue_tool_delta_events(&mut state, &deltas(&arguments)).unwrap();
    let queued = events(&state);
    let ten = "\u{e9}".repeat(10);
    let first = format!(
        r#"{{"type":"response.function_call_arguments.delta","item_id":"fc_c","output_index":1,"delta":"{ten}","sequence_number":2}}"#
    );
    let second = "{\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_c\",\"output_index\":1,\"delta\":\"\u{e9}\",\"sequence_number\":3}";
    assert_eq!(queued[1], json(first.as_str()));
    assert_eq!(queued[2], json(second));
}

#[test]
fn final_tool_completion_emits_missing_added_and_remaining_arguments() {
    let mut state = state_without_tools();
    queue_tool_delta_events(&mut state, &deltas("ab")).unwrap();
    queue_final_tool_completion(
        &mut state,
        &json(
            r#"{"choices":[{"message":{"tool_calls":[{"id":"c","function":{"name":"f","arguments":"XYZ"}}]}}]}"#,
        ),
    )
    .unwrap();
    let queued = events(&state);
    assert_eq!(queued.len(), 5);
    assert_eq!(
        queued[2],
        json(
            r#"{"type":"response.function_call_arguments.delta","item_id":"fc_c","output_index":1,"delta":"Z","sequence_number":3}"#
        )
    );
    assert_eq!(
        queued[3],
        json(
            r#"{"type":"response.function_call_arguments.done","item_id":"fc_c","output_index":1,"arguments":"XYZ","sequence_number":4}"#
        )
    );
    assert_eq!(
        queued[4],
        json(
            r#"{"type":"response.output_item.done","output_index":1,"sequence_number":5,"item":{"type":"function_call","id":"fc_c","call_id":"c","name":"f","status":"completed","arguments":"XYZ"}}"#
        )
    );
}

#[test]
fn final_tool_completion_adds_a_call_that_never_streamed() {
    let mut state = state_without_tools();
    queue_final_tool_completion(
        &mut state,
        &json(
            r#"{"choices":[{"message":{"tool_calls":[{"id":"c","function":{"name":"f","arguments":"12345678901"}}]}}]}"#,
        ),
    )
    .unwrap();
    let queued = events(&state);
    assert_eq!(queued.len(), 5);
    assert_eq!(
        queued[0],
        json(
            r#"{"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","id":"fc_c","call_id":"c","name":"f","status":"in_progress","arguments":""},"sequence_number":1}"#
        )
    );
    assert_eq!(
        queued[4],
        json(
            r#"{"type":"response.output_item.done","output_index":1,"sequence_number":5,"item":{"type":"function_call","id":"fc_c","call_id":"c","name":"f","status":"completed","arguments":"12345678901"}}"#
        )
    );
}

#[test]
fn final_tool_completion_falls_back_to_streamed_arguments() {
    let mut state = state_without_tools();
    queue_tool_delta_events(&mut state, &deltas("ab")).unwrap();
    queue_final_tool_completion(
        &mut state,
        &json(
            r#"{"choices":[{"message":{"tool_calls":[{"id":"c","function":{"name":"f","arguments":""}}]}}]}"#,
        ),
    )
    .unwrap();
    let queued = events(&state);
    assert_eq!(
        queued[2],
        json(
            r#"{"type":"response.function_call_arguments.done","item_id":"fc_c","output_index":1,"arguments":"ab","sequence_number":3}"#
        )
    );
    assert_eq!(
        queued[3],
        json(
            r#"{"type":"response.output_item.done","output_index":1,"sequence_number":4,"item":{"type":"function_call","id":"fc_c","call_id":"c","name":"f","status":"completed","arguments":"ab"}}"#
        )
    );
}

#[test]
fn final_tool_completion_emits_done_events_for_empty_arguments() {
    let mut state = state_without_tools();
    queue_final_tool_completion(
        &mut state,
        &json(
            r#"{"choices":[{"message":{"tool_calls":[{"id":"c","function":{"name":"f","arguments":""}}]}}]}"#,
        ),
    )
    .unwrap();
    let queued = events(&state);
    assert_eq!(queued.len(), 3);
    assert_eq!(
        queued[1],
        json(
            r#"{"type":"response.function_call_arguments.done","item_id":"fc_c","output_index":1,"arguments":"","sequence_number":2}"#
        )
    );
}

#[rstest]
#[case::no_choices(r#"{"choices":[]}"#)]
#[case::not_a_list(r#"{"choices":[{"message":{"tool_calls":{"a":1}}}]}"#)]
#[case::empty_list(r#"{"choices":[{"message":{"tool_calls":[]}}]}"#)]
#[case::no_message("{}")]
fn final_tool_completion_ignores_completions_without_tool_calls(#[case] completion: &str) {
    let mut state = state_without_tools();
    queue_final_tool_completion(&mut state, &json(completion)).unwrap();
    assert!(events(&state).is_empty());
    assert!(state.progress.final_tool_events_queued);
    queue_final_tool_completion(
        &mut state,
        &json(
            r#"{"choices":[{"message":{"tool_calls":[{"id":"c","function":{"name":"f","arguments":"1"}}]}}]}"#,
        ),
    )
    .unwrap();
    assert!(events(&state).is_empty());
}

#[test]
fn final_tool_completion_runs_only_once() {
    let mut state = state_without_tools();
    let completion = json(
        r#"{"choices":[{"message":{"tool_calls":[{"id":"c","function":{"name":"f","arguments":"1"}}]}}]}"#,
    );
    queue_final_tool_completion(&mut state, &completion).unwrap();
    let first = events(&state);
    queue_final_tool_completion(&mut state, &completion).unwrap();
    assert_eq!(events(&state), first);
}

#[test]
fn final_tool_completion_keeps_the_custom_item_shape() {
    let mut state = state_with(json(r#"[{"type":"custom","name":"patch"}]"#));
    queue_tool_delta_events(
        &mut state,
        &json(r#"[{"id":"c","index":0,"function":{"name":"patch","arguments":"{\"content\":\"x\"}"}}]"#),
    )
    .unwrap();
    queue_final_tool_completion(
        &mut state,
        &json(
            r#"{"choices":[{"message":{"tool_calls":[{"id":"c","function":{"name":"patch","arguments":"{\"content\":\"xy\"}"}}]}}]}"#,
        ),
    )
    .unwrap();
    let queued = events(&state);
    let last = queued.last().expect("the done event is queued");
    assert_eq!(
        last,
        &json(
            r#"{"type":"response.output_item.done","output_index":1,"sequence_number":6,"item":{"type":"custom_tool_call","id":"ctc_c","call_id":"c","name":"patch","status":"completed","input":"xy"}}"#
        )
    );
}
