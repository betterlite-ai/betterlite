use betterlite_models::decisions::Json;

use crate::api::messages_stream_split::errors::StreamFailure;

use super::fixtures::{Reply, drain, drain_async, driver, field, json, kinds, sequence};

const REASONING_A: &str = r#"{"id":"chat_a","choices":[{"index":0,"delta":{"reasoning_content":"a"},"finish_reason":null}]}"#;
const REASONING_B: &str = r#"{"id":"chat_a","choices":[{"index":0,"delta":{"reasoning_content":"b"},"finish_reason":null}]}"#;
const TEXT_HI: &str =
    r#"{"id":"chat_a","choices":[{"index":0,"delta":{"content":"hi"},"finish_reason":null}]}"#;
const FINISH_STOP: &str =
    r#"{"id":"chat_a","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#;
const REASONING_AND_TEXT: &str = r#"{"id":"chat_a","choices":[{"index":0,"delta":{"reasoning_content":"x","content":"y"},"finish_reason":null}]}"#;

fn event_type(event: &Json) -> Json {
    field(event, "type")
}

fn reasoning_chunks() -> Vec<Result<Json, StreamFailure>> {
    [REASONING_A, REASONING_B, TEXT_HI, FINISH_STOP]
        .iter()
        .map(|text| Ok(json(text)))
        .collect()
}

async fn reasoning_events() -> Vec<Json> {
    let (mut stream, _received) = driver(
        reasoning_chunks(),
        Reply::Text,
        "{}",
        &["r1", "u1", "u2", "u3"],
    );
    drain_async(&mut stream).await
}

#[tokio::test]
async fn driver_async_reasoning_closes_before_the_new_content() {
    let events = reasoning_events().await;
    assert_eq!(
        kinds(&events),
        [
            "response.created",
            "response.in_progress",
            "response.output_item.added",
            "response.reasoning_summary_text.delta",
            "response.reasoning_summary_text.delta",
            "response.reasoning_summary_text.done",
            "response.reasoning_summary_part.done",
            "response.output_item.done",
            "response.output_item.added",
            "response.content_part.added",
            "response.output_text.delta",
            "response.output_text.done",
            "response.content_part.done",
            "response.output_item.done",
            "response.completed",
        ]
    );
    assert_eq!(field(&events[3], "delta"), Json::Text(String::from("a")));
    assert_eq!(field(&events[5], "text"), Json::Text(String::from("ab")));
    assert_eq!(
        field(&field(&events[6], "part"), "text"),
        Json::Text(String::from("ab")),
        "the summary part joins the fragments without a separator"
    );
    assert_eq!(
        field(&field(&events[2], "item"), "type"),
        Json::Text(String::from("reasoning"))
    );
}

#[tokio::test]
async fn driver_async_reasoning_sequences_and_item_payloads() {
    let events = reasoning_events().await;
    let sequences: Vec<Option<i64>> = events.iter().map(sequence).collect();
    assert_eq!(
        sequences,
        [
            Some(1),
            Some(2),
            Some(3),
            None,
            None,
            Some(4),
            Some(5),
            Some(6),
            Some(7),
            Some(8),
            Some(9),
            None,
            None,
            Some(1),
            None,
        ]
    );
    assert_eq!(
        field(&field(&events[2], "item"), "id"),
        Json::Text(String::from("rs_r1")),
        "the reasoning item takes its id from the stream's id source"
    );
    assert_eq!(
        field(&events[5], "item_id"),
        Json::Text(String::from("rs_r1")),
        "the done events keep the same reasoning item id"
    );
    assert_eq!(
        field(&field(&events[2], "item"), "summary"),
        Json::Null,
        "the item starts without a summary"
    );
}

#[tokio::test]
async fn driver_async_reasoning_item_precedes_the_message() {
    let events = reasoning_events().await;
    assert_eq!(
        field(&events[8], "output_index"),
        json("1"),
        "the message item follows the reasoning item"
    );
    let response = field(&events[14], "response");
    let Json::List(output) = field(&response, "output") else {
        panic!("the completed response carries output items");
    };
    let Json::Object(message) = &output[0] else {
        panic!("the first output item is an object");
    };
    assert_eq!(
        message.get("id"),
        Some(&Json::Text(String::from("msg_u1"))),
        "the completed response keeps the streamed item id"
    );
}

#[test]
fn driver_sync_reasoning_close_is_absent() {
    let (mut stream, _received) = driver(
        reasoning_chunks(),
        Reply::Text,
        "{}",
        &["r1", "u1", "u2", "u3"],
    );
    let events = drain(&mut stream);
    let seen = kinds(&events);
    assert!(seen.contains(&String::from("response.reasoning_summary_text.delta")));
    assert!(!seen.contains(&String::from("response.reasoning_summary_text.done")));
    assert!(!seen.contains(&String::from("response.reasoning_summary_part.done")));
    let item_done = events
        .iter()
        .filter(|event| event_type(event) == Json::Text(String::from("response.output_item.done")))
        .count();
    assert_eq!(item_done, 1, "only the message item finishes");
}

#[test]
fn driver_tool_first_chunk_creates_no_message_shell() {
    let (mut stream, _received) = driver(
        vec![Ok(json(
            r#"{"id":"chat_a","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"lookup","arguments":"{\"x\":1}"}}]},"finish_reason":null}]}"#,
        ))],
        Reply::ToolCall,
        "{}",
        &["u1"],
    );
    let events = drain(&mut stream);
    assert_eq!(
        kinds(&events),
        [
            "response.created",
            "response.in_progress",
            "response.output_item.added",
            "response.function_call_arguments.delta",
            "response.function_call_arguments.done",
            "response.output_item.done",
            "response.completed",
        ]
    );
    let added = field(&events[2], "item");
    assert_eq!(
        field(&added, "type"),
        Json::Text(String::from("function_call"))
    );
    assert_eq!(field(&events[2], "output_index"), json("1"));
}

#[test]
fn driver_signed_thinking_block_starts_reasoning() {
    let (mut stream, _received) = driver(
        vec![Ok(json(
            r#"{"id":"chat_a","choices":[{"index":0,"delta":{"thinking_blocks":[{"type":"thinking","signature":"sig"}]},"finish_reason":null}]}"#,
        ))],
        Reply::None,
        "{}",
        &["r1"],
    );
    let events = drain(&mut stream);
    assert_eq!(
        kinds(&events),
        [
            "response.created",
            "response.in_progress",
            "response.output_item.added",
        ]
    );
    assert_eq!(
        field(&field(&events[2], "item"), "type"),
        Json::Text(String::from("reasoning"))
    );
}

#[test]
fn driver_unsigned_thinking_block_is_not_reasoning() {
    let (mut stream, _received) = driver(
        vec![Ok(json(
            r#"{"id":"chat_a","choices":[{"index":0,"delta":{"thinking_blocks":[{"type":"thinking","signature":""}]},"finish_reason":null}]}"#,
        ))],
        Reply::None,
        "{}",
        &["u1"],
    );
    let events = drain(&mut stream);
    assert_eq!(
        kinds(&events),
        [
            "response.created",
            "response.in_progress",
            "response.output_item.added",
            "response.content_part.added",
        ],
        "an unsigned thinking block leaves the default message item"
    );
}

#[test]
fn driver_reasoning_wins_over_text() {
    let (mut stream, _received) = driver(
        vec![Ok(json(REASONING_AND_TEXT)), Ok(json(FINISH_STOP))],
        Reply::Text,
        "{}",
        &["r1", "u1", "u2", "u3"],
    );
    let events = drain(&mut stream);
    let deltas: Vec<Json> = events
        .iter()
        .filter(|event| {
            event_type(event) == Json::Text(String::from("response.reasoning_summary_text.delta"))
        })
        .map(|event| field(event, "delta"))
        .collect();
    assert_eq!(deltas, [Json::Text(String::from("x"))]);
    assert!(
        !kinds(&events).contains(&String::from("response.output_text.delta")),
        "the text of the same chunk waits for a later chunk"
    );
}

#[test]
fn driver_reasoning_keeps_the_text_in_the_snapshot() {
    let (mut stream, received) = driver(
        vec![Ok(json(REASONING_AND_TEXT)), Ok(json(FINISH_STOP))],
        Reply::Text,
        "{}",
        &["r1", "u1", "u2", "u3"],
    );
    drain(&mut stream);
    let chunks = received.lock().expect("the calls lock").clone();
    let Some(Json::List(choices)) = chunks[0][0].get("choices") else {
        panic!("the snapshot keeps the choices");
    };
    let Json::Object(choice) = &choices[0] else {
        panic!("the snapshot choice is an object");
    };
    let Some(Json::Object(delta)) = choice.get("delta") else {
        panic!("the snapshot keeps the delta");
    };
    assert_eq!(
        (delta.get("content"), delta.get("reasoning_content")),
        (
            Some(&Json::Text(String::from("y"))),
            Some(&Json::Text(String::from("x")))
        )
    );
}

#[test]
fn driver_empty_reasoning_emits_no_delta() {
    let (mut stream, _received) = driver(
        vec![Ok(json(
            r#"{"id":"chat_a","choices":[{"index":0,"delta":{"reasoning_content":""},"finish_reason":null}]}"#,
        ))],
        Reply::None,
        "{}",
        &["u1"],
    );
    let events = drain(&mut stream);
    assert_eq!(
        kinds(&events),
        [
            "response.created",
            "response.in_progress",
            "response.output_item.added",
            "response.content_part.added",
        ],
        "an empty reasoning fragment leaves the default message item"
    );
}

#[test]
fn driver_empty_text_emits_no_delta() {
    let (mut stream, _received) = driver(
        vec![Ok(json(
            r#"{"id":"chat_a","choices":[{"index":0,"delta":{"content":""},"finish_reason":null}]}"#,
        ))],
        Reply::None,
        "{}",
        &["u1"],
    );
    let events = drain(&mut stream);
    assert_eq!(
        kinds(&events),
        [
            "response.created",
            "response.in_progress",
            "response.output_item.added",
            "response.content_part.added",
        ]
    );
}
