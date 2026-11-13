use betterlite_models::decisions::Json;
use rstest::rstest;

use super::super::error::StreamToolsError;
use super::super::final_tools::queue_final_tool_completion;
use super::super::state::{ChatStreamState, assign_tool_output_index};
use super::super::tool_calls::queue_tool_delta_events;
use super::super::web_search::reserve_web_search_indexes;
use super::fixtures::{deltas, events, json, state_with};
use super::state_without_tools;

#[test]
fn hosted_search_reservation_strips_one_prefix_and_shares_the_tool_index() {
    let mut state = state_with(json(r#"[{"type":"function","name":"f"}]"#));
    reserve_web_search_indexes(
        &mut state,
        &json(
            r#"{"web_search_calls":{"a":{"id":"ws_call_x","status":"completed","action":{"type":"search"}}}}"#,
        ),
    )
    .unwrap();
    assert_eq!(assign_tool_output_index(&mut state, "call_x").unwrap(), 1);
    assert!(state.progress.web_search_calls.contains_key("call_x"));
}

#[test]
fn hosted_search_reservation_accepts_a_list() {
    let mut state = state_without_tools();
    reserve_web_search_indexes(
        &mut state,
        &json(
            r#"{"web_search_calls":[{"id":"ws_one","status":"searching"},{"id":"ws_two","status":"completed"}]}"#,
        ),
    )
    .unwrap();
    assert_eq!(assign_tool_output_index(&mut state, "one").unwrap(), 1);
    assert_eq!(assign_tool_output_index(&mut state, "two").unwrap(), 2);
}

#[test]
fn hosted_search_reservation_keeps_the_latest_item() {
    let mut state = state_without_tools();
    reserve_web_search_indexes(
        &mut state,
        &json(r#"{"web_search_calls":{"a":{"id":"ws_call_x","status":"searching"}}}"#),
    )
    .unwrap();
    reserve_web_search_indexes(
        &mut state,
        &json(r#"{"web_search_calls":{"later":{"id":"ws_call_x","status":"failed"}}}"#),
    )
    .unwrap();
    let stored = state
        .progress
        .web_search_calls
        .get("call_x")
        .expect("the item stays");
    assert_eq!(stored, &json(r#"{"id":"ws_call_x","status":"failed"}"#));
}

#[test]
fn hosted_search_reservation_skips_empty_ids() {
    let mut state = state_without_tools();
    reserve_web_search_indexes(
        &mut state,
        &json(
            r#"{"web_search_calls":{"a":{"id":"ws_call_x","status":"completed"},"b":{"status":"completed"},"c":{"id":"ws_"},"d":"text"}}"#,
        ),
    )
    .unwrap();
    assert_eq!(state.progress.web_search_calls.len(), 1);
    assert!(state.progress.web_search_calls.contains_key("call_x"));
}

#[rstest]
#[case::not_an_object("null")]
#[case::no_calls(r#"{"other":1}"#)]
#[case::text_calls(r#"{"web_search_calls":"x"}"#)]
fn hosted_search_reservation_ignores_other_shapes(#[case] provider_fields: &str) {
    let mut state = state_without_tools();
    reserve_web_search_indexes(&mut state, &json(provider_fields)).unwrap();
    assert!(state.progress.web_search_calls.is_empty());
    assert_eq!(state.progress.next_tool_output_index, 1);
}

#[test]
fn hosted_search_reservation_keeps_pending_events_for_other_statuses() {
    let mut state = state_with(json(r#"[{"type":"function","name":"f"}]"#));
    queue_tool_delta_events(
        &mut state,
        &json(r#"[{"id":"call_x","index":0,"function":{"name":"f","arguments":"abcdefghijkl"}}]"#),
    )
    .unwrap();
    reserve_web_search_indexes(
        &mut state,
        &json(r#"{"web_search_calls":{"a":{"id":"ws_call_x","status":"searching"}}}"#),
    )
    .unwrap();
    assert_eq!(events(&state).len(), 3);
}

#[test]
fn hosted_search_reservation_removes_pending_events_while_in_progress() {
    let mut state = state_with(json(r#"[{"type":"function","name":"f"}]"#));
    queue_tool_delta_events(
        &mut state,
        &json(r#"[{"id":"call_x","index":0,"function":{"name":"f","arguments":"abcdefghijkl"}}]"#),
    )
    .unwrap();
    reserve_web_search_indexes(
        &mut state,
        &json(r#"{"web_search_calls":{"a":{"id":"ws_call_x","status":"in_progress"}}}"#),
    )
    .unwrap();
    assert!(events(&state).is_empty());
}

fn reserve_searching(state: &mut ChatStreamState<String>) {
    reserve_web_search_indexes(
        state,
        &json(
            r#"{"web_search_calls":{"k":{"id":"ws_call_x","status":"searching","type":"web_search_call","action":{"type":"open_page"}}}}"#,
        ),
    )
    .unwrap();
}

#[test]
fn hosted_search_final_emits_an_added_call_and_progress() {
    let mut state = state_without_tools();
    reserve_searching(&mut state);
    queue_final_tool_completion(
        &mut state,
        &json(
            r#"{"choices":[{"message":{"tool_calls":[{"id":"call_x","function":{"name":"web_search","arguments":"{}"}}]}}]}"#,
        ),
    )
    .unwrap();
    let queued = events(&state);
    assert_eq!(queued.len(), 5);
    assert_eq!(
        queued[0],
        json(
            r#"{"type":"response.output_item.added","output_index":1,"item":{"id":"ws_call_x","type":"web_search_call","status":"in_progress","action":null},"sequence_number":1}"#
        )
    );
    assert_eq!(
        queued[1],
        json(
            r#"{"type":"response.web_search_call.in_progress","output_index":1,"item_id":"ws_call_x","sequence_number":2}"#
        )
    );
    assert_eq!(
        queued[2],
        json(
            r#"{"type":"response.web_search_call.searching","output_index":1,"item_id":"ws_call_x","sequence_number":3}"#
        )
    );
    assert!(state.progress.queued_web_search_call_ids.contains("call_x"));
}

#[test]
fn hosted_search_final_carries_the_final_item() {
    let mut state = state_without_tools();
    reserve_searching(&mut state);
    queue_final_tool_completion(
        &mut state,
        &json(
            r#"{"choices":[{"message":{"tool_calls":[{"id":"call_x","function":{"name":"web_search","arguments":"{}"}}]}}]}"#,
        ),
    )
    .unwrap();
    let queued = events(&state);
    assert_eq!(
        queued[3],
        json(
            r#"{"type":"response.web_search_call.completed","output_index":1,"item_id":"ws_call_x","sequence_number":4}"#
        )
    );
    assert_eq!(
        queued[4],
        json(
            r#"{"type":"response.output_item.done","output_index":1,"sequence_number":5,"item":{"type":"web_search_call","id":"ws_call_x","action":{"type":"open_page","url":null},"status":"searching"}}"#
        )
    );
}

#[test]
fn hosted_search_final_emits_a_known_call_once() {
    let mut state = state_without_tools();
    reserve_web_search_indexes(
        &mut state,
        &json(
            r#"{"web_search_calls":{"k":{"id":"ws_call_x","status":"completed","type":"web_search_call","action":{"type":"find_in_page","pattern":"p","url":"u"}}}}"#,
        ),
    )
    .unwrap();
    queue_final_tool_completion(
        &mut state,
        &json(
            r#"{"choices":[{"message":{"tool_calls":[{"id":"call_x","function":{"name":"web_search","arguments":"{}"}},{"id":"call_x","function":{"name":"web_search","arguments":"{}"}}]}}]}"#,
        ),
    )
    .unwrap();
    assert_eq!(events(&state).len(), 5);
}

#[test]
fn hosted_search_final_rejects_an_invalid_record() {
    let mut state = state_without_tools();
    reserve_web_search_indexes(
        &mut state,
        &json(
            r#"{"web_search_calls":{"k":{"id":"ws_call_x","status":"weird","action":{"type":"open_page"}}}}"#,
        ),
    )
    .unwrap();
    let result = queue_final_tool_completion(
        &mut state,
        &json(
            r#"{"choices":[{"message":{"tool_calls":[{"id":"call_x","function":{"name":"web_search","arguments":"{}"}}]}}]}"#,
        ),
    );
    assert!(matches!(result, Err(StreamToolsError::WebSearch { .. })));
    assert!(events(&state).is_empty());
}

#[rstest]
#[case::missing_action(r#"{"id":"ws_x","type":"web_search_call","status":"completed"}"#)]
#[case::unknown_action(
    r#"{"id":"ws_x","type":"web_search_call","status":"completed","action":{"type":"jump"}}"#
)]
#[case::wrong_type(
    r#"{"id":"ws_x","type":"web_search","status":"completed","action":{"type":"open_page"}}"#
)]
#[case::wrong_id(
    r#"{"id":5,"type":"web_search_call","status":"completed","action":{"type":"open_page"}}"#
)]
fn hosted_search_final_rejects_invalid_items(#[case] item: &str) {
    let mut state = state_without_tools();
    state
        .progress
        .web_search_calls
        .insert(String::from("x"), json(item));
    let result = queue_final_tool_completion(
        &mut state,
        &json(
            r#"{"choices":[{"message":{"tool_calls":[{"id":"x","function":{"name":"web_search","arguments":"{}"}}]}}]}"#,
        ),
    );
    assert!(matches!(result, Err(StreamToolsError::WebSearch { .. })));
}

#[test]
fn hosted_search_final_keeps_extra_fields_from_the_record() {
    let mut state = state_without_tools();
    reserve_web_search_indexes(
        &mut state,
        &json(
            r#"{"web_search_calls":{"k":{"id":"ws_x","status":"completed","type":"web_search_call","action":{"type":"search","query":"q","provider":"p"},"vendor":7}}}"#,
        ),
    )
    .unwrap();
    queue_final_tool_completion(
        &mut state,
        &json(
            r#"{"choices":[{"message":{"tool_calls":[{"id":"x","function":{"name":"web_search","arguments":"{}"}}]}}]}"#,
        ),
    )
    .unwrap();
    let queued = events(&state);
    let last = queued.last().expect("the done event is queued");
    assert_eq!(
        last,
        &json(
            r#"{"type":"response.output_item.done","output_index":1,"sequence_number":5,"item":{"type":"web_search_call","id":"ws_x","action":{"type":"search","queries":null,"query":"q","sources":null,"provider":"p"},"status":"completed","vendor":7}}"#
        )
    );
}

#[test]
fn tool_calls_input_is_never_mistaken_for_a_web_search_call() {
    let mut state = state_without_tools();
    reserve_web_search_indexes(
        &mut state,
        &json(
            r#"{"web_search_calls":{"k":{"id":"call_x","status":"completed","action":{"type":"open_page"}}}}"#,
        ),
    )
    .unwrap();
    queue_tool_delta_events(
        &mut state,
        &json(r#"[{"id":"call_x","index":0,"function":{"name":"f","arguments":""}}]"#),
    )
    .unwrap();
    assert!(events(&state).is_empty());
}

#[test]
fn sequence_numbers_continue_across_queues() {
    let mut state = state_without_tools();
    queue_tool_delta_events(&mut state, &deltas("a")).unwrap();
    reserve_web_search_indexes(
        &mut state,
        &json(
            r#"{"web_search_calls":{"k":{"id":"ws_call_x","status":"completed","type":"web_search_call","action":{"type":"open_page"}}}}"#,
        ),
    )
    .unwrap();
    queue_final_tool_completion(
        &mut state,
        &json(
            r#"{"choices":[{"message":{"tool_calls":[{"id":"call_x","function":{"name":"web_search"}},{"id":"c","function":{"name":"f","arguments":"a"}}]}}]}"#,
        ),
    )
    .unwrap();
    let queued = events(&state);
    assert_eq!(queued.len(), 9);
    let sequences: Vec<Json> = queued
        .into_iter()
        .filter_map(|event| match event {
            Json::Object(fields) => fields.get("sequence_number").cloned(),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
        })
        .collect();
    let expected: Vec<Json> = (1_i64..=9)
        .map(|number| Json::Number(serde_json::Number::from(number)))
        .collect();
    assert_eq!(sequences, expected);
}
