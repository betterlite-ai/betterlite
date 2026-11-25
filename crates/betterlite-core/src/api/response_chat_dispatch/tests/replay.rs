use betterlite_models::decisions::Json;
use betterlite_streaming::stream_chunk_shaping::records::{JsonValue, Usage};

use super::fixtures::support::{
    FIXED_CLOCK, FakeExecution, chat_json, chat_response, delta_member_of, hidden_member_of,
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
    assert_eq!(chat_json(&expect_chat(output)), chat_json(&chat_response()));
}

#[tokio::test]
async fn a_passed_through_chat_answer_keeps_its_choice_logprobs() {
    let mut body = chat_response();
    body.choices[0].logprobs =
        Some(serde_json::from_str(r#"{"content":[{"token":"hello","logprob":-0.25}]}"#).unwrap());
    let execution = FakeExecution::new(Answer::Chat(body.clone()));
    let output = expect_ok(completion(&execution, call(&kwargs("gpt-4o-mini", "openai"))).await);
    assert_eq!(chat_json(&expect_chat(output)), chat_json(&body));
}

// LiteLLM's Message always dumps content, role, function_call, tool_calls and provider_specific_fields, null when unset
fn body_with(raw_message: &str) -> Box<super::ChatResponse> {
    let raw = format!(
        r#"{{"id":"chatcmpl-x","object":"chat.completion","created":1,"model":"gpt-4o-mini","choices":[{{"index":0,"message":{raw_message},"finish_reason":"stop"}}]}}"#
    );
    Box::new(serde_json::from_str(&raw).unwrap())
}

fn message_of(output: &super::ChatResponse) -> serde_json::Value {
    serde_json::to_value(&output.choices[0].message).unwrap()
}

#[tokio::test]
async fn a_passed_through_chat_answer_keeps_its_images_annotations_and_reasoning_items() {
    let raw = r#"{"content":"hi","role":"assistant","tool_calls":null,"function_call":null,"images":[{"image_url":{"url":"https://img.test/a.png","detail":"high"},"index":3,"type":"image_url"},{"image_url":{"url":"https://img.test/b.png"},"index":4,"type":"image_url"}],"reasoning_items":[{"type":"reasoning","id":"rs_1","encrypted_content":"cipher","summary":[{"type":"summary_text","text":"thought"}]},{"type":"reasoning","id":"rs_2","summary":[{"type":"summary_text"}]}],"provider_specific_fields":null,"annotations":[{"type":"url_citation","url_citation":{"start_index":0,"end_index":2,"title":"A","url":"https://a.test"}},{"type":"url_citation","url_citation":{"url":"https://b.test"}}]}"#;
    let execution = FakeExecution::new(Answer::Chat(body_with(raw)));
    let output = expect_chat(expect_ok(
        completion(&execution, call(&kwargs("gpt-4o-mini", "openai"))).await,
    ));
    let message = &output.choices[0].message;
    let images = message.images.as_ref().expect("the image stays typed");
    assert_eq!(images[0].index, 3);
    assert_eq!(images[0].image_url.url, "https://img.test/a.png");
    assert_eq!(images[0].image_url.detail.as_deref(), Some("high"));
    assert_eq!(images[1].image_url.detail, None);
    let annotations = message
        .annotations
        .as_ref()
        .expect("the annotation stays typed");
    let citation = annotations[0]
        .url_citation
        .as_ref()
        .expect("the annotation carries its citation");
    assert_eq!(citation.url.as_deref(), Some("https://a.test"));
    assert_eq!(citation.start_index, Some(0));
    assert_eq!(citation.end_index, Some(2));
    assert_eq!(annotations[1].url_citation.as_ref().unwrap().title, None);
    let reasoning = message
        .reasoning_items
        .as_ref()
        .expect("the reasoning item stays typed");
    assert_eq!(reasoning[0].id.as_deref(), Some("rs_1"));
    assert_eq!(reasoning[0].encrypted_content.as_deref(), Some("cipher"));
    assert_eq!(
        reasoning[0].summary.as_ref().expect("the summary stays")[0]
            .text
            .as_deref(),
        Some("thought")
    );
    assert_eq!(reasoning[1].summary.as_ref().unwrap()[0].text, None);
    assert!(message.extra.is_empty());
    assert_eq!(
        message_of(&output),
        serde_json::from_str::<serde_json::Value>(raw).unwrap()
    );
}

#[tokio::test]
async fn a_passed_through_chat_answer_keeps_a_reasoning_item_without_a_summary() {
    let raw = r#"{"content":"hi","role":"assistant","tool_calls":null,"function_call":null,"reasoning_items":[{"type":"reasoning","id":"rs_1","encrypted_content":"cipher"}],"provider_specific_fields":null}"#;
    let execution = FakeExecution::new(Answer::Chat(body_with(raw)));
    let output = expect_chat(expect_ok(
        completion(&execution, call(&kwargs("gpt-4o-mini", "openai"))).await,
    ));
    let reasoning = &output.choices[0]
        .message
        .reasoning_items
        .as_ref()
        .expect("the reasoning item stays typed")[0];
    assert!(reasoning.summary.is_none());
    assert_eq!(
        message_of(&output),
        serde_json::from_str::<serde_json::Value>(raw).unwrap()
    );
}

#[tokio::test]
async fn a_passed_through_chat_answer_keeps_an_unknown_tool_call_field() {
    let raw = r#"{"content":null,"role":"assistant","tool_calls":[{"id":"call_1","type":"function","index":2,"function":{"name":"get_weather","arguments":"{}"}}],"function_call":null,"provider_specific_fields":null}"#;
    let execution = FakeExecution::new(Answer::Chat(body_with(raw)));
    let output = expect_chat(expect_ok(
        completion(&execution, call(&kwargs("gpt-4o-mini", "openai"))).await,
    ));
    let carried = output.choices[0]
        .message
        .tool_calls
        .as_ref()
        .expect("the tool call stays")[0]
        .extra
        .get("index")
        .and_then(JsonValue::as_i64);
    assert_eq!(carried, Some(2));
    assert_eq!(
        message_of(&output),
        serde_json::from_str::<serde_json::Value>(raw).unwrap()
    );
}

#[tokio::test]
async fn a_passed_through_chat_answer_keeps_its_message_audio_and_thinking_blocks() {
    let mut body = chat_response();
    body.choices[0].message.audio = Some(
        serde_json::from_str(r#"{"id":"audio-1","data":"AAA","expires_at":1,"transcript":"hi"}"#)
            .unwrap(),
    );
    body.choices[0].message.thinking_blocks =
        Some(serde_json::from_str(r#"[{"type":"thinking","signature":"sig-1"}]"#).unwrap());
    let execution = FakeExecution::new(Answer::Chat(body.clone()));
    let output = expect_ok(completion(&execution, call(&kwargs("gpt-4o-mini", "openai"))).await);
    assert_eq!(chat_json(&expect_chat(output)), chat_json(&body));
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
    body.choices[0].message.content = None;
    body.choices[0].message.tool_calls = Some(
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
    body.choices[0].message.audio = Some(
        serde_json::from_str(r#"{"id":"audio-1","data":"AAA","expires_at":1,"transcript":"hi"}"#)
            .unwrap(),
    );
    body.choices[0].message.thinking_blocks =
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
async fn a_replayed_chunk_names_the_provider_response_model_in_its_hidden_params() {
    let execution = FakeExecution::new(Answer::Chat(chat_response()));
    let chunks = drain(expect_stream(expect_ok(
        completion(&execution, call(&streaming())).await,
    )))
    .await;
    assert_eq!(
        hidden_member_of(&chunks[0], "provider_response_model"),
        Some(Json::Text(String::from("gpt-4o-mini")))
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
