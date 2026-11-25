use betterlite_models::decisions::Json;
use betterlite_streaming::stream_chunk_shaping::records::Usage;

use super::fixtures::support::{
    FIXED_CLOCK, FakeExecution, chat_response, delta_member_of, hidden_member_of,
};
use super::{
    Answer, call, completion, created_of, drain, expect_chat, expect_ok, expect_stream, finishes,
    ids, joined, kwargs, streaming,
};

fn contains_text(value: &Json, needle: &str) -> bool {
    match value {
        Json::Text(text) => text.contains(needle),
        Json::List(items) => items.iter().any(|item| contains_text(item, needle)),
        Json::Object(fields) => fields.values().any(|item| contains_text(item, needle)),
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
async fn a_passed_through_chat_answer_keeps_its_choice_logprobs() {
    let mut body = chat_response();
    body.choices[0].logprobs =
        Some(serde_json::from_str(r#"{"content":[{"token":"hello","logprob":-0.25}]}"#).unwrap());
    let execution = FakeExecution::new(Answer::Chat(body.clone()));
    let output = expect_ok(completion(&execution, call(&kwargs("gpt-4o-mini", "openai"))).await);
    assert_eq!(expect_chat(output), body);
}

#[tokio::test]
async fn a_passed_through_chat_answer_keeps_its_message_audio_and_thinking_blocks() {
    let mut body = chat_response();
    body.choices[0].message.parts.audio =
        Some(serde_json::from_str(r#"{"id":"audio-1","data":"AAA"}"#).unwrap());
    body.choices[0].message.parts.thinking_blocks =
        Some(serde_json::from_str(r#"[{"type":"thinking","signature":"sig-1"}]"#).unwrap());
    let execution = FakeExecution::new(Answer::Chat(body.clone()));
    let output = expect_ok(completion(&execution, call(&kwargs("gpt-4o-mini", "openai"))).await);
    assert_eq!(expect_chat(output), body);
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
    body.choices[0].finish_reason = Some(String::from("tool_calls"));
    body.choices[0].message.parts.content = None;
    body.choices[0].message.parts.tool_calls = Some(
        serde_json::from_str(
            r#"[{"id":"call_1","type":"function","function":{"name":"get_weather","arguments":"{\"city\":\"Paris\"}"}}]"#,
        )
        .unwrap(),
    );
    let execution = FakeExecution::new(Answer::Chat(body));
    let chunks = drain(expect_stream(expect_ok(
        completion(&execution, call(&streaming())).await,
    )))
    .await;
    assert_eq!(finishes(&chunks), vec![String::from("tool_calls")]);
    let calls = delta_member_of(&chunks[0], "tool_calls")
        .expect("the replayed chunk carries the tool calls");
    assert!(contains_text(&calls, "call_1"));
    assert!(contains_text(&calls, "get_weather"));
    assert!(contains_text(&calls, "Paris"));
}

#[tokio::test]
async fn a_replayed_response_keeps_its_length_finish_reason() {
    let mut body = chat_response();
    body.choices[0].finish_reason = Some(String::from("length"));
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
        serde_json::from_str::<Usage>(
            r#"{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}"#,
        )
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
async fn a_replay_carries_message_audio_and_thinking_blocks_into_the_delta() {
    let mut body = chat_response();
    body.choices[0].message.parts.audio =
        Some(serde_json::from_str(r#"{"id":"audio-1"}"#).unwrap());
    body.choices[0].message.parts.thinking_blocks =
        Some(serde_json::from_str(r#"[{"type":"thinking","signature":"sig-1"}]"#).unwrap());
    let execution = FakeExecution::new(Answer::Chat(body));
    let chunks = drain(expect_stream(expect_ok(
        completion(&execution, call(&streaming())).await,
    )))
    .await;
    let audio = delta_member_of(&chunks[0], "audio").expect("the replayed delta carries the audio");
    let blocks = delta_member_of(&chunks[0], "thinking_blocks")
        .expect("the replayed delta carries the thinking blocks");
    assert!(contains_text(&audio, "audio-1"));
    assert!(contains_text(&blocks, "sig-1"));
}

#[tokio::test]
async fn a_replay_keeps_an_empty_id_and_takes_the_wrapper_clock() {
    let mut body = chat_response();
    body.id = String::new();
    body.created = 0;
    body.choices[0].message.parts.content = Some(String::from("hey"));
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
