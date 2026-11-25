use betterlite_models::decisions::Json;

use super::fixtures::support::{
    FIXED_CLOCK, FakeExecution, chat_response, hidden_member_of, tool_calls_of,
};
use super::{
    Answer, call, completion, created_of, drain, expect_chat, expect_ok, expect_stream, finishes,
    ids, joined, kwargs, streaming,
};
use crate::api::responses_tool_results::records::{FunctionChunk, ToolCallChunk};

fn contains_text(value: &Json, needle: &str) -> bool {
    match value {
        Json::Text(text) => text.contains(needle),
        Json::List(items) => items.iter().any(|item| contains_text(item, needle)),
        Json::Object(fields) => fields.values().any(|value| contains_text(value, needle)),
        Json::Null | Json::Bool(_) | Json::Number(_) => false,
    }
}

#[tokio::test]
async fn a_direct_model_response_passes_through_when_not_streaming() {
    let execution = FakeExecution::new(Answer::Chat(chat_response()));
    let output = expect_ok(completion(&execution, call(&kwargs("gpt-4o-mini", "openai"))).await);
    assert_eq!(expect_chat(output), chat_response());
}

#[tokio::test]
async fn a_direct_model_response_streams_through_the_mock_iterator() {
    let execution = FakeExecution::new(Answer::Chat(chat_response()));
    let chunks = drain(expect_stream(expect_ok(
        completion(&execution, call(&streaming())).await,
    )))
    .await;
    assert_eq!(joined(&chunks), "hello");
    assert_eq!(finishes(&chunks), vec![String::from("stop")]);
    assert!(ids(&chunks).iter().all(|id| id == "chatcmpl-x"));
    assert!(!ids(&chunks).is_empty());
    // the wrapper builds its own chunk, so the replayed created makes way for the wrapper's clock
    assert_eq!(
        created_of(&chunks[0]),
        Some(Json::Number(serde_json::Number::from(FIXED_CLOCK)))
    );
}

#[tokio::test]
async fn a_replayed_response_keeps_its_tool_calls_and_finish_reason() {
    let mut body = chat_response();
    body.choices[0].finish_reason = String::from("tool_calls");
    body.choices[0].message.content = None;
    body.choices[0].message.tool_calls = Some(vec![ToolCallChunk {
        id: String::from("call_1"),
        kind: String::from("function"),
        function: FunctionChunk {
            name: String::from("get_weather"),
            arguments: String::from(r#"{"city":"Paris"}"#),
            provider_specific_fields: None,
        },
        index: 0,
        provider_specific_fields: None,
    }]);
    let execution = FakeExecution::new(Answer::Chat(body));
    let chunks = drain(expect_stream(expect_ok(
        completion(&execution, call(&streaming())).await,
    )))
    .await;
    assert_eq!(finishes(&chunks), vec![String::from("tool_calls")]);
    let calls = tool_calls_of(&chunks[0]).expect("the replayed chunk carries the tool calls");
    assert!(contains_text(&calls, "call_1"));
    assert!(contains_text(&calls, "get_weather"));
    assert!(contains_text(&calls, "Paris"));
}

#[tokio::test]
async fn a_replayed_response_keeps_its_length_finish_reason() {
    let mut body = chat_response();
    body.choices[0].finish_reason = String::from("length");
    let execution = FakeExecution::new(Answer::Chat(body));
    let chunks = drain(expect_stream(expect_ok(
        completion(&execution, call(&streaming())).await,
    )))
    .await;
    assert_eq!(finishes(&chunks), vec![String::from("length")]);
}

#[tokio::test]
async fn a_replayed_response_carries_its_usage_into_the_stream_totals() {
    let mut body = chat_response();
    body.usage = Some(
        serde_json::from_str(r#"{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}"#)
            .unwrap(),
    );
    let execution = FakeExecution::new(Answer::Chat(body));
    let chunks = drain(expect_stream(expect_ok(
        completion(&execution, call(&streaming())).await,
    )))
    .await;
    let closing = chunks.last().expect("the wrapper closes the stream");
    let usage =
        hidden_member_of(closing, "usage").expect("the closing chunk carries the usage total");
    let Json::Object(fields) = usage else {
        panic!("the usage total is an object");
    };
    assert_eq!(
        fields.get("total_tokens"),
        Some(&Json::Number(serde_json::Number::from(3_i64)))
    );
}

#[tokio::test]
async fn a_replay_keeps_an_empty_id_and_takes_the_wrapper_clock() {
    let mut body = chat_response();
    body.id = String::new();
    body.created = 0;
    body.choices[0].message.content = Some(String::from("hey"));
    let execution = FakeExecution::new(Answer::Chat(body));
    let chunks = drain(expect_stream(expect_ok(
        completion(&execution, call(&streaming())).await,
    )))
    .await;
    assert_eq!(joined(&chunks), "hey");
    assert!(ids(&chunks).iter().all(String::is_empty));
    assert_eq!(
        created_of(&chunks[0]),
        Some(Json::Number(serde_json::Number::from(FIXED_CLOCK)))
    );
}

#[tokio::test]
async fn a_replay_without_choices_closes_the_stream_itself() {
    let mut body = chat_response();
    body.choices.clear();
    let execution = FakeExecution::new(Answer::Chat(body));
    let chunks = drain(expect_stream(expect_ok(
        completion(&execution, call(&streaming())).await,
    )))
    .await;
    assert!(joined(&chunks).is_empty());
    assert_eq!(finishes(&chunks), vec![String::from("stop")]);
}
