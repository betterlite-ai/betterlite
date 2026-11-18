use betterlite_core::api::realtime_contracts::events::AllDeltaType;
use betterlite_models::decisions::Json;

use super::support;
use crate::realtime_events::decode::json_dict;
use crate::realtime_events::error::EventError;

const CONTENT_START: &str = r#"{"event":{"contentStart":{"role":"ASSISTANT","type":"TEXT"}}}"#;
const USAGE_EVENT: &str = r#"{"event":{"usageEvent":{"totalInputTokens":10,"totalOutputTokens":4,"totalTokens":14,"details":{"total":{"input":{"speechTokens":2,"textTokens":8},"output":{"speechTokens":1,"textTokens":3}}}}}}"#;
const USAGE_JSON: &str = r#"{"input_tokens":10,"output_tokens":4,"total_tokens":14,"input_token_details":{"audio_tokens":2,"text_tokens":8,"cached_tokens":0},"output_token_details":{"audio_tokens":1,"text_tokens":3}}"#;
const ZERO_USAGE_JSON: &str = r#"{"input_tokens":0,"output_tokens":0,"total_tokens":0,"input_token_details":{"audio_tokens":0,"text_tokens":0,"cached_tokens":0},"output_token_details":{"audio_tokens":0,"text_tokens":0}}"#;

pub(super) fn native_completion_dispatch() {
    content_start_events();
    text_deltas_and_end_turn();
    content_end_variants();
    usage_delta();
    usage_array_rejected();
    tool_use_output();
    prompt_end_state();
}

// A payload of the wrong shape is an error and leaves the cumulative counters as they were
fn usage_array_rejected() {
    let ids = support::CountingIds::new();
    let mut session = support::new_session();
    let mut input = support::input();
    let response =
        support::transform(&mut session, &ids, CONTENT_START, &input).expect("a content start");
    input = support::next_input(&response);
    support::transform(&mut session, &ids, USAGE_EVENT, &input).expect("a usage event");
    let error = support::transform(&mut session, &ids, r#"{"event":{"usageEvent":[]}}"#, &input)
        .expect_err("an array usage event");
    assert!(matches!(
        error,
        EventError::Payload {
            field: "usageEvent",
            ..
        }
    ));
    let response = support::transform(&mut session, &ids, r#"{"event":{"promptEnd":{}}}"#, &input)
        .expect("a prompt end");
    let done = json_dict(&support::events(&response)[0]).expect("a response.done object");
    let object = done
        .get("response")
        .and_then(json_dict)
        .expect("a response object");
    assert_eq!(object.get("usage"), Some(&support::json(USAGE_JSON)));
}

fn content_start_events() {
    let ids = support::CountingIds::new();
    let mut session = support::new_session();
    let response = support::transform(&mut session, &ids, CONTENT_START, &support::input())
        .expect("an assistant content start");
    let events = support::events(&response);
    assert_eq!(events.len(), 3);
    assert_eq!(support::event_type(&events[0]), Some("response.created"));
    assert_eq!(
        support::event_type(&events[1]),
        Some("response.output_item.added")
    );
    assert_eq!(
        support::event_type(&events[2]),
        Some("response.content_part.added")
    );
    assert_eq!(
        response.current_output_item_id.as_deref(),
        Some(support::numbered("item", 2).as_str())
    );
    assert_eq!(
        response.current_response_id.as_deref(),
        Some(support::numbered("resp", 1).as_str())
    );
    assert_eq!(
        response.current_conversation_id.as_deref(),
        Some(support::numbered("conv", 3).as_str())
    );
    assert_eq!(response.current_delta_type, Some(AllDeltaType::Text));
    let created = json_dict(&events[0]).expect("a response.created object");
    assert_eq!(
        created.get("event_id"),
        Some(&Json::Text(support::numbered("event", 4)))
    );
}

fn text_deltas_and_end_turn() {
    let ids = support::CountingIds::new();
    let mut session = support::new_session();
    let mut input = support::input();
    let response =
        support::transform(&mut session, &ids, CONTENT_START, &input).expect("a content start");
    input = support::next_input(&response);
    let response = support::transform(
        &mut session,
        &ids,
        r#"{"event":{"textOutput":{"content":"Hello"}}}"#,
        &input,
    )
    .expect("a text delta");
    assert_eq!(support::events(&response).len(), 1);
    assert!(response.current_delta_chunks.is_some());
    input = support::next_input(&response);
    let response = support::transform(
        &mut session,
        &ids,
        r#"{"event":{"textOutput":{"content":" world"}}}"#,
        &input,
    )
    .expect("a text delta");
    input = support::next_input(&response);
    let response = support::transform(
        &mut session,
        &ids,
        r#"{"event":{"contentEnd":{"stopReason":"END_TURN"}}}"#,
        &input,
    )
    .expect("a content end");
    let events = support::events(&response);
    assert_eq!(events.len(), 4);
    assert_eq!(support::event_type(&events[0]), Some("response.text.done"));
    let done = json_dict(&events[0]).expect("a text done object");
    assert_eq!(
        done.get("text"),
        Some(&Json::Text("Hello world".to_owned()))
    );
    assert_eq!(
        done.get("event_id"),
        Some(&Json::Text(support::numbered("event", 7)))
    );
    assert_eq!(
        support::event_type(&events[1]),
        Some("response.content_part.done")
    );
    assert_eq!(
        support::event_type(&events[2]),
        Some("response.output_item.done")
    );
    assert_eq!(support::event_type(&events[3]), Some("response.done"));
    assert_eq!(response.current_output_item_id, None);
    assert_eq!(response.current_response_id, None);
    assert_eq!(response.current_delta_type, None);
    assert_eq!(response.current_delta_chunks, None);
    assert_eq!(
        response.current_conversation_id.as_deref(),
        Some(support::numbered("conv", 3).as_str())
    );
}

fn content_end_variants() {
    let mut input = support::input_with_ids("item", "resp", "conv");
    input.current_delta_type = Some(AllDeltaType::Text);
    input.current_delta_chunks = Some(vec![support::json(r#"{"delta":"Hi"}"#)]);
    for message in [
        r#"{"event":{"contentEnd":{"stopReason":"MAX_TOKENS"}}}"#,
        r#"{"event":{"contentEnd":{}}}"#,
        r#"{"event":{"contentEnd":{"stopReason":"end_turn"}}}"#,
    ] {
        let events = support::transform_events(message, &input);
        assert_eq!(events.len(), 3);
        assert_eq!(support::event_type(&events[0]), Some("response.text.done"));
    }
    let events = support::transform_events(
        r#"{"event":{"contentEnd":{"stopReason":"END_TURN"}}}"#,
        &input,
    );
    assert_eq!(events.len(), 4);
    missing_ids();
    partial_ids();
    invalid_payload();
}

// A content end that closed nothing closeable leaves the accumulated chunks as they came
fn partial_ids() {
    for (item, response) in [(Some("item"), None), (None, Some("resp"))] {
        let mut input = support::input();
        input.current_output_item_id = item.map(str::to_owned);
        input.current_response_id = response.map(str::to_owned);
        input.current_delta_type = Some(AllDeltaType::Text);
        input.current_delta_chunks = Some(vec![support::json(r#"{"delta":"Hi"}"#)]);
        let result = support::transform(
            &mut support::new_session(),
            &support::CountingIds::new(),
            r#"{"event":{"contentEnd":{"stopReason":"MAX_TOKENS"}}}"#,
            &input,
        )
        .expect("a content end with one id");
        assert_eq!(support::events(&result).len(), 0);
        assert_eq!(result.current_delta_chunks, input.current_delta_chunks);
    }
}

fn missing_ids() {
    let mut input = support::input();
    input.current_delta_type = Some(AllDeltaType::Text);
    input.current_delta_chunks = Some(vec![support::json(r#"{"delta":"Hi"}"#)]);
    let ids = support::CountingIds::new();
    let mut session = support::new_session();
    let response = support::transform(
        &mut session,
        &ids,
        r#"{"event":{"contentEnd":{"stopReason":"END_TURN"}}}"#,
        &input,
    )
    .expect("a content end");
    assert_eq!(support::events(&response).len(), 0);
    assert_eq!(response.current_delta_chunks, input.current_delta_chunks);
    assert_eq!(response.current_output_item_id, None);
    assert_eq!(response.current_response_id, None);
}

fn invalid_payload() {
    let input = support::input_with_ids("item", "resp", "conv");
    let response = support::transform(
        &mut support::new_session(),
        &support::CountingIds::new(),
        r#"{"event":{"contentEnd":3}}"#,
        &input,
    );
    assert!(matches!(
        response,
        Err(EventError::Payload {
            field: "contentEnd",
            ..
        })
    ));
}

fn usage_delta() {
    let ids = support::CountingIds::new();
    let mut session = support::new_session();
    let mut input = support::input();
    let response =
        support::transform(&mut session, &ids, CONTENT_START, &input).expect("a content start");
    input = support::next_input(&response);
    support::transform(&mut session, &ids, USAGE_EVENT, &input).expect("a usage event");
    let response = support::transform(&mut session, &ids, r#"{"event":{"promptEnd":{}}}"#, &input)
        .expect("a prompt end");
    let events = support::events(&response);
    assert_eq!(events.len(), 1);
    let done = json_dict(&events[0]).expect("a response.done object");
    let object = done
        .get("response")
        .and_then(json_dict)
        .expect("a response object");
    assert_eq!(object.get("usage"), Some(&support::json(USAGE_JSON)));
    input = support::next_input(&response);
    let response =
        support::transform(&mut session, &ids, CONTENT_START, &input).expect("a content start");
    input = support::next_input(&response);
    let response = support::transform(&mut session, &ids, r#"{"event":{"promptEnd":{}}}"#, &input)
        .expect("a prompt end");
    let events = support::events(&response);
    let done = json_dict(&events[0]).expect("a response.done object");
    let object = done
        .get("response")
        .and_then(json_dict)
        .expect("a response object");
    assert_eq!(object.get("usage"), Some(&support::json(ZERO_USAGE_JSON)));
}

fn tool_use_output() {
    let (log, _guard) = support::capture();
    let ids = support::CountingIds::new();
    let mut session = support::new_session();
    let input = support::input_with_ids("item", "resp", "conv");
    let message = r#"{"event":{"toolUse":{"toolUseId":"call-1","toolName":"lookup","input":"{\"city\":\"Paris\"}"}}}"#;
    let response =
        support::transform(&mut session, &ids, message, &input).expect("a tool use event");
    let events = support::events(&response);
    assert_eq!(events.len(), 1);
    assert_eq!(
        support::event_type(&events[0]),
        Some("response.function_call_arguments.done")
    );
    let done = json_dict(&events[0]).expect("a function call object");
    assert_eq!(done.get("call_id"), Some(&Json::Text("call-1".to_owned())));
    assert_eq!(done.get("name"), Some(&Json::Text("lookup".to_owned())));
    assert_eq!(
        done.get("arguments"),
        Some(&Json::Text(r#"{"city":"Paris"}"#.to_owned()))
    );
    assert!(log.text().contains("Tool use event: lookup (ID: call-1)"));
    let response = support::transform(&mut session, &ids, message, &support::input())
        .expect("a tool use event without ids");
    assert_eq!(support::events(&response).len(), 0);
    assert!(log.text().contains("Tool use event:  (ID: )"));
}

fn prompt_end_state() {
    let mut input = support::input_with_ids("item", "resp", "conv");
    input.current_delta_type = Some(AllDeltaType::Audio);
    input.current_delta_chunks = Some(vec![support::json(r#"{"delta":"a"}"#)]);
    input.current_item_chunks = Some(vec![support::json(r#"{"delta":"b"}"#)]);
    for message in [
        r#"{"event":{"promptEnd":{}}}"#,
        r#"{"event":{"completionEnd":{}}}"#,
    ] {
        let ids = support::CountingIds::new();
        let mut session = support::new_session();
        let response =
            support::transform(&mut session, &ids, message, &input).expect("a response end");
        assert_eq!(support::events(&response).len(), 1);
        assert_eq!(
            support::event_type(&support::events(&response)[0]),
            Some("response.done")
        );
        assert_eq!(response.current_output_item_id, None);
        assert_eq!(response.current_response_id, None);
        assert_eq!(response.current_delta_type, None);
        assert_eq!(response.current_delta_chunks, input.current_delta_chunks);
        assert_eq!(response.current_item_chunks, input.current_item_chunks);
        assert_eq!(response.current_conversation_id.as_deref(), Some("conv"));
    }
}
