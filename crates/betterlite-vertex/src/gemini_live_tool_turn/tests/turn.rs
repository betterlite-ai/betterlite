#![cfg(test)]

use serde_json::json;

use super::support::{empty_state, frame, given_ids, object, run_turn, tool_call};
use crate::gemini_live_tool_turn::turn::ToolTurnInput;

#[test]
fn two_calls_emit_five_correlated_events_each_then_one_done() {
    let mut state = empty_state();
    let mut server_frame = frame("{}");
    let calls = tool_call(
        r#"{"functionCalls": [{"id": "a", "name": "first", "args": {"x": 1}}, {"id": "b", "name": "second", "args": {}}]}"#,
    );
    let turn = run_turn(&mut state, &mut server_frame, &calls, &given_ids());
    let events = serde_json::to_value(&turn.events).expect("the events serialize");
    let event_types: Vec<&str> = events
        .as_array()
        .expect("the events serialize as an array")
        .iter()
        .map(|event| event["type"].as_str().expect("every event carries a type"))
        .collect();
    assert_eq!(
        event_types,
        [
            "response.output_item.added",
            "conversation.item.added",
            "response.function_call_arguments.delta",
            "response.function_call_arguments.done",
            "response.output_item.done",
            "response.output_item.added",
            "conversation.item.added",
            "response.function_call_arguments.delta",
            "response.function_call_arguments.done",
            "response.output_item.done",
            "response.done",
        ]
    );
    assert_eq!(events[0]["item"]["id"], "item_given_tool_0");
    assert_eq!(events[0]["item"]["status"], "in_progress");
    assert_eq!(events[0]["item"]["arguments"], "");
    assert_eq!(events[1]["previous_item_id"], json!(null));
    assert_eq!(events[2]["delta"], "{\"x\": 1}");
    assert_eq!(events[3]["call_id"], "a");
    assert_eq!(events[3]["arguments"], "{\"x\": 1}");
}

#[test]
fn a_following_call_continues_the_output_index_and_own_arguments() {
    let mut state = empty_state();
    let mut server_frame = frame("{}");
    let calls = tool_call(
        r#"{"functionCalls": [{"id": "a", "name": "first", "args": {"x": 1}}, {"id": "b", "name": "second", "args": {}}]}"#,
    );
    let turn = run_turn(&mut state, &mut server_frame, &calls, &given_ids());
    let events = serde_json::to_value(&turn.events).expect("the events serialize");
    assert_eq!(events[5]["output_index"], 1_i32);
    assert_eq!(events[5]["item"]["id"], "item_given_tool_1");
    assert_eq!(events[5]["item"]["arguments"], "");
    assert_eq!(events[6]["item"]["call_id"], "b");
    assert_eq!(events[7]["call_id"], "b");
    assert_eq!(events[7]["delta"], "{}");
    assert_eq!(events[9]["item"]["status"], "completed");
    assert_eq!(events[9]["item"]["arguments"], "{}");
}

#[test]
fn a_tool_turn_keeps_the_conversation_resets_turn_ids_and_records_call_names() {
    let mut state = empty_state();
    let mut server_frame = frame("{}");
    let calls = tool_call(
        r#"{"functionCalls": [{"id": "a", "name": "first", "args": {"x": 1}}, {"id": "b", "name": "second", "args": {}}]}"#,
    );
    let turn = run_turn(&mut state, &mut server_frame, &calls, &given_ids());
    let events = serde_json::to_value(&turn.events).expect("the events serialize");
    assert_eq!(
        events[10]["response"]["output"].as_array().map(Vec::len),
        Some(2)
    );
    assert_eq!(events[10]["response"]["id"], "resp_given");
    assert_eq!(events[10]["response"]["modalities"], json!(["audio"]));
    assert!(turn.response_id.is_none());
    assert!(turn.output_item_id.is_none());
    assert_eq!(turn.conversation_id.as_deref(), Some("conv_given"));
    assert_eq!(
        state.tool_call_names.entries.get("b").map(String::as_str),
        Some("second")
    );
}

#[test]
fn a_fresh_turn_creates_response_and_conversation_with_drawn_ids() {
    let mut state = empty_state();
    let mut server_frame = frame("{}");
    let calls = tool_call(r#"{"functionCalls": [{"id": "a", "name": "first"}]}"#);
    let turn = run_turn(
        &mut state,
        &mut server_frame,
        &calls,
        &ToolTurnInput::default(),
    );
    let events = serde_json::to_value(&turn.events).expect("the events serialize");
    let created = &events[0]["response"];
    assert_eq!(events[0]["type"], "response.created");
    assert_eq!(
        events[0]["event_id"],
        "event_04040404-0404-0404-0404-040404040404"
    );
    assert_eq!(
        created["conversation_id"],
        "conv_01010101-0101-0101-0101-010101010101"
    );
    assert_eq!(created["id"], "resp_02020202-0202-0202-0202-020202020202");
    assert_eq!(created["status"], "in_progress");
    assert_eq!(created["output"], json!([]));
    assert_eq!(created["modalities"], json!(["audio"]));
    assert_eq!(created["temperature"], json!(null));
    assert_eq!(created["max_output_tokens"], json!(null));
    assert_eq!(
        events[1]["item"]["id"],
        "item_03030303-0303-0303-0303-030303030303_tool_0"
    );
    assert_eq!(
        events[1]["event_id"],
        "event_06060606-0606-0606-0606-060606060606"
    );
    assert_eq!(events[6]["type"], "response.done");
    assert_eq!(
        events[6]["event_id"],
        "event_0a0a0a0a-0a0a-0a0a-0a0a-0a0a0a0a0a0a"
    );
}

#[test]
fn empty_calls_leave_ids_names_and_pending_usage_alone() {
    let mut state = empty_state();
    state.pending_usage_metadata = Some(object(r#"{"promptTokenCount": 4}"#));
    let mut server_frame = frame("{}");
    let calls = tool_call(r#"{"functionCalls": []}"#);
    let turn = run_turn(&mut state, &mut server_frame, &calls, &given_ids());
    assert!(turn.events.is_empty());
    assert_eq!(turn.conversation_id.as_deref(), Some("conv_given"));
    assert_eq!(turn.response_id.as_deref(), Some("resp_given"));
    assert_eq!(turn.output_item_id.as_deref(), Some("item_given"));
    assert!(state.tool_call_names.entries.is_empty());
    assert!(state.pending_usage_metadata.is_some());
}

#[test]
fn an_existing_response_without_an_item_base_generates_one() {
    let mut state = empty_state();
    let mut server_frame = frame("{}");
    let calls = tool_call(
        r#"{"functionCalls": [{"id": "a", "name": "first"}, {"id": "b", "name": "second"}]}"#,
    );
    let input = ToolTurnInput {
        conversation_id: None,
        response_id: Some("resp_given"),
        output_item_id: None,
        session_configuration_request: None,
    };
    let turn = run_turn(&mut state, &mut server_frame, &calls, &input);
    let events = serde_json::to_value(&turn.events).expect("the events serialize");
    assert_eq!(events[0]["type"], "response.output_item.added");
    assert_eq!(
        events[0]["item"]["id"],
        "item_02020202-0202-0202-0202-020202020202_tool_0"
    );
    assert_eq!(
        events[5]["item"]["id"],
        "item_02020202-0202-0202-0202-020202020202_tool_1"
    );
    assert_eq!(events[10]["response"]["id"], "resp_given");
    assert_eq!(
        events[10]["response"]["output"].as_array().map(Vec::len),
        Some(2)
    );
}

#[test]
fn frame_usage_is_billed_once_without_grounding_counters() {
    let mut state = empty_state();
    state.pending_usage_metadata = Some(object(r#"{"promptTokenCount": 99}"#));
    let mut server_frame = frame(
        r#"{"usageMetadata": {"promptTokenCount": 10, "toolUsePromptTokenCount": 3, "candidatesTokenCount": 5, "totalTokenCount": 18}, "serverContent": {"groundingMetadata": {"webSearchQueries": ["q"]}}}"#,
    );
    let calls = tool_call(r#"{"functionCalls": [{"id": "a", "name": "first"}]}"#);
    let turn = run_turn(&mut state, &mut server_frame, &calls, &given_ids());
    let events = serde_json::to_value(&turn.events).expect("the events serialize");
    let usage = &events[5]["response"]["usage"];
    assert_eq!(usage["input_tokens"], 13_i32);
    assert_eq!(usage["output_tokens"], 5_i32);
    assert!(
        usage["input_token_details"]
            .get("web_search_requests")
            .is_none()
    );
    assert!(state.pending_usage_metadata.is_none());
    assert!(server_frame.usage_metadata.is_none());
}

#[test]
fn pending_usage_is_billed_when_the_frame_carries_none() {
    let mut state = empty_state();
    state.pending_usage_metadata = Some(object(
        r#"{"promptTokenCount": 7, "candidatesTokenCount": 2, "totalTokenCount": 9}"#,
    ));
    let mut server_frame = frame("{}");
    let calls = tool_call(r#"{"functionCalls": [{"id": "a", "name": "first"}]}"#);
    let turn = run_turn(&mut state, &mut server_frame, &calls, &given_ids());
    let events = serde_json::to_value(&turn.events).expect("the events serialize");
    assert_eq!(events[5]["response"]["usage"]["input_tokens"], 7_i32);
    assert!(state.pending_usage_metadata.is_none());
}

#[test]
fn setup_supplies_lowered_modalities_and_the_limit_pair() {
    let mut state = empty_state();
    let mut server_frame = frame("{}");
    let calls = tool_call(r#"{"functionCalls": [{"id": "a", "name": "first"}]}"#);
    let input = ToolTurnInput {
        conversation_id: None,
        response_id: None,
        output_item_id: None,
        session_configuration_request: Some(
            r#"{"setup": {"generationConfig": {"temperature": 0.7, "maxOutputTokens": 100, "responseModalities": ["TEXT", "Audio"]}}}"#,
        ),
    };
    let turn = run_turn(&mut state, &mut server_frame, &calls, &input);
    let events = serde_json::to_value(&turn.events).expect("the events serialize");
    assert_eq!(
        events[0]["response"]["modalities"],
        json!(["text", "audio"])
    );
    assert_eq!(events[0]["response"]["temperature"], 0.7_f64);
    assert_eq!(events[0]["response"]["max_output_tokens"], 100_i32);
    assert_eq!(
        events[6]["response"]["modalities"],
        json!(["text", "audio"])
    );
    assert_eq!(events[6]["response"]["temperature"], 0.7_f64);
    assert_eq!(events[6]["response"]["max_output_tokens"], 100_i32);
}

#[test]
fn an_unparsable_setup_falls_back_to_audio_without_limits() {
    let mut state = empty_state();
    let mut server_frame = frame("{}");
    let calls = tool_call(r#"{"functionCalls": [{"id": "a", "name": "first"}]}"#);
    let input = ToolTurnInput {
        conversation_id: None,
        response_id: None,
        output_item_id: None,
        session_configuration_request: Some("not json"),
    };
    let turn = run_turn(&mut state, &mut server_frame, &calls, &input);
    let events = serde_json::to_value(&turn.events).expect("the events serialize");
    assert_eq!(events[0]["response"]["modalities"], json!(["audio"]));
    assert_eq!(events[0]["response"]["temperature"], json!(null));
    assert!(events[6]["response"].get("temperature").is_none());
}
