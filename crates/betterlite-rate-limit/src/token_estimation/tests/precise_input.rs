#![cfg(test)]

use std::sync::Mutex;

use super::estimate::{TokenEstimateRequest, estimate_precise_input_tokens};
use super::support::{
    FixedCounter, FixedResponses, FixedTranslation, RecordingCounter, SeenCount, chat_reply,
    no_translation, request, value,
};

fn estimate_with<'request>(
    data: &'request super::RequestData,
    call_type: Option<&'request str>,
    model: Option<&'request str>,
) -> TokenEstimateRequest<'request> {
    TokenEstimateRequest {
        data,
        call_type,
        model,
        min_configured_tpm_limit: None,
        configured_output_tokens: None,
    }
}

fn recording_counter(tokens: Option<i64>) -> RecordingCounter {
    RecordingCounter {
        tokens,
        seen: Mutex::new(Vec::new()),
    }
}

fn first_count(counter: &RecordingCounter) -> SeenCount {
    let seen = counter.seen.lock().expect("the counter recording lock");
    seen[0].clone()
}

#[tokio::test]
async fn responses_messages_reach_the_counter() {
    let data = request(r#"{"input":"hi"}"#);
    let responses = FixedResponses {
        messages: vec![value(r#"{"role":"user","content":"hi"}"#)],
    };
    let counter = recording_counter(Some(7));
    let total = estimate_precise_input_tokens(
        &no_translation(),
        &responses,
        &counter,
        &estimate_with(&data, Some("responses"), None),
    )
    .await;
    assert_eq!(total, 7);
    let seen = first_count(&counter);
    assert_eq!(
        seen.messages,
        Some(vec![value(r#"{"role":"user","content":"hi"}"#)])
    );
    assert_eq!(seen.text, None);
}

#[tokio::test]
async fn pretokenized_embedding_input_skips_the_counter() {
    let data = request(r#"{"input":[1,2,3]}"#);
    let counter = recording_counter(Some(99));
    let total = estimate_precise_input_tokens(
        &no_translation(),
        &FixedResponses {
            messages: Vec::new(),
        },
        &counter,
        &estimate_with(&data, Some("embedding"), None),
    )
    .await;
    assert_eq!(total, 3);
    assert!(
        counter
            .seen
            .lock()
            .expect("the counter recording lock")
            .is_empty()
    );
}

#[tokio::test]
async fn embedding_text_reaches_the_counter() {
    let data = request(r#"{"input":"hello"}"#);
    let counter = recording_counter(Some(4));
    let total = estimate_precise_input_tokens(
        &no_translation(),
        &FixedResponses {
            messages: Vec::new(),
        },
        &counter,
        &estimate_with(&data, Some("embedding"), None),
    )
    .await;
    assert_eq!(total, 4);
    let seen = first_count(&counter);
    assert_eq!(seen.text, Some(value(r#""hello""#)));
    assert_eq!(seen.messages, None);
}

#[tokio::test]
async fn rerank_text_joins_the_query_and_the_documents() {
    let data = request(r#"{"query":"q","documents":["a","b"]}"#);
    let counter = recording_counter(Some(5));
    let _total = estimate_precise_input_tokens(
        &no_translation(),
        &FixedResponses {
            messages: Vec::new(),
        },
        &counter,
        &estimate_with(&data, Some("rerank"), None),
    )
    .await;
    assert_eq!(first_count(&counter).text, Some(value(r#""q\na\nb""#)));
}

#[tokio::test]
async fn a_text_completion_passes_the_prompt() {
    let data = request(r#"{"prompt":"hello"}"#);
    let counter = recording_counter(Some(5));
    let _total = estimate_precise_input_tokens(
        &no_translation(),
        &FixedResponses {
            messages: Vec::new(),
        },
        &counter,
        &estimate_with(&data, Some("text_completion"), None),
    )
    .await;
    assert_eq!(first_count(&counter).text, Some(value(r#""hello""#)));
}

#[tokio::test]
async fn chat_messages_reach_the_counter() {
    let data = request(r#"{"messages":[{"role":"user","content":"hi"}]}"#);
    let counter = recording_counter(Some(5));
    let _total = estimate_precise_input_tokens(
        &no_translation(),
        &FixedResponses {
            messages: Vec::new(),
        },
        &counter,
        &estimate_with(&data, Some("completion"), None),
    )
    .await;
    assert_eq!(
        first_count(&counter).messages,
        Some(vec![value(r#"{"role":"user","content":"hi"}"#)])
    );
}

#[tokio::test]
async fn tools_and_the_tool_choice_reach_the_counter() {
    let data = request(
        r#"{"messages":[],"tools":[{"type":"function","function":{"name":"f"}}],"tool_choice":"auto"}"#,
    );
    let counter = recording_counter(Some(5));
    let _total = estimate_precise_input_tokens(
        &no_translation(),
        &FixedResponses {
            messages: Vec::new(),
        },
        &counter,
        &estimate_with(&data, Some("completion"), None),
    )
    .await;
    let seen = first_count(&counter);
    assert_eq!(
        seen.tools,
        Some(vec![value(
            r#"{"type":"function","function":{"name":"f"}}"#
        )])
    );
    assert_eq!(seen.tool_choice, Some(value(r#""auto""#)));
}

#[tokio::test]
async fn the_model_reaches_the_counter() {
    let data = request(r#"{"messages":[]}"#);
    let counter = recording_counter(Some(5));
    let _total = estimate_precise_input_tokens(
        &no_translation(),
        &FixedResponses {
            messages: Vec::new(),
        },
        &counter,
        &estimate_with(&data, Some("completion"), Some("gpt-4")),
    )
    .await;
    assert_eq!(first_count(&counter).model, "gpt-4");
}

#[tokio::test]
async fn a_missing_model_reaches_the_counter_as_empty_text() {
    let data = request(r#"{"messages":[]}"#);
    let counter = recording_counter(Some(5));
    let _total = estimate_precise_input_tokens(
        &no_translation(),
        &FixedResponses {
            messages: Vec::new(),
        },
        &counter,
        &estimate_with(&data, Some("completion"), None),
    )
    .await;
    assert_eq!(first_count(&counter).model, "");
}

#[tokio::test]
async fn audio_blocks_are_stripped_before_counting_and_added_back() {
    let data = request(
        r#"{"messages":[{"role":"user","content":[{"type":"input_audio","input_audio":{}},{"type":"text","text":"hi"}]}]}"#,
    );
    let counter = recording_counter(Some(10));
    let total = estimate_precise_input_tokens(
        &no_translation(),
        &FixedResponses {
            messages: Vec::new(),
        },
        &counter,
        &estimate_with(&data, Some("completion"), None),
    )
    .await;
    assert_eq!(total, 310);
    assert_eq!(
        first_count(&counter).messages,
        Some(vec![value(
            r#"{"role":"user","content":[{"type":"text","text":"hi"}]}"#
        )])
    );
}

#[tokio::test]
async fn a_failed_count_falls_back_to_the_char_estimate_with_audio() {
    let data = request(
        r#"{"messages":[{"role":"user","content":[{"type":"input_audio","input_audio":{}},{"type":"text","text":"abcdefgh"}]}]}"#,
    );
    let counter = recording_counter(None);
    let total = estimate_precise_input_tokens(
        &no_translation(),
        &FixedResponses {
            messages: Vec::new(),
        },
        &counter,
        &estimate_with(&data, Some("completion"), None),
    )
    .await;
    assert_eq!(total, 302);
}

#[tokio::test]
async fn a_failed_rerank_count_falls_back_to_the_text_length() {
    let data = request(r#"{"query":"abcdefgh"}"#);
    let counter = recording_counter(None);
    let total = estimate_precise_input_tokens(
        &no_translation(),
        &FixedResponses {
            messages: Vec::new(),
        },
        &counter,
        &estimate_with(&data, Some("rerank"), None),
    )
    .await;
    assert_eq!(total, 2);
}

#[tokio::test]
async fn a_failed_chat_count_falls_back_to_the_char_estimate() {
    let data = request(r#"{"messages":[{"role":"user","content":"abcdefgh"}]}"#);
    let counter = recording_counter(None);
    let total = estimate_precise_input_tokens(
        &no_translation(),
        &FixedResponses {
            messages: Vec::new(),
        },
        &counter,
        &estimate_with(&data, Some("completion"), None),
    )
    .await;
    assert_eq!(total, 2);
}

#[tokio::test]
async fn translated_google_messages_reach_the_counter() {
    let data = request(r#"{"contents":[{"parts":[{"text":"hi"}]}]}"#);
    let translation = FixedTranslation {
        reply: Some(chat_reply(r#"[{"role":"user","content":"hi"}]"#)),
    };
    let counter = FixedCounter { tokens: Some(9) };
    let total = estimate_precise_input_tokens(
        &translation,
        &FixedResponses {
            messages: Vec::new(),
        },
        &counter,
        &estimate_with(&data, Some("generate_content"), None),
    )
    .await;
    assert_eq!(total, 9);
}
