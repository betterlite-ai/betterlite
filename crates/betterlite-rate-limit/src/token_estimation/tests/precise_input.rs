#![cfg(test)]

use std::sync::Mutex;

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::estimate::{TokenEstimateRequest, estimate_precise_input_tokens};
use super::services::TranslatedChat;
use super::support::{
    FixedResponses, FixedTranslation, RecordingCounter, SeenCount, chat_reply, no_translation,
    request, value,
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
async fn a_failed_non_rerank_count_does_not_use_the_rerank_text_length() {
    let data = request(r#"{"prompt":"abcdefgh"}"#);
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
    assert_eq!(total, 0);
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
    let counter = recording_counter(Some(9));
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
    let seen = first_count(&counter);
    assert_eq!(
        seen.messages,
        Some(vec![value(r#"{"role":"user","content":"hi"}"#)])
    );
    assert_eq!(seen.text, None);
}

#[rstest]
#[case::translated_tools(
    Some(r#"[{"type":"function","function":{"name":"translated"}}]"#),
    Some(r#""required""#)
)]
#[case::no_translated_tools(None, None)]
#[tokio::test]
async fn a_translated_google_request_passes_the_translated_tools_and_tool_choice(
    #[case] tools: Option<&str>,
    #[case] tool_choice: Option<&str>,
) {
    let data = request(
        r#"{"contents":[{"parts":[{"text":"hi"}]}],"tools":[{"functionDeclarations":[{"name":"raw"}]}],"toolConfig":{"functionCallingConfig":{"mode":"AUTO"}},"tool_choice":"auto"}"#,
    );
    let translation = FixedTranslation {
        reply: Some(TranslatedChat {
            messages: value(r#"[{"role":"user","content":"hi"}]"#),
            tools: tools.map(value),
            tool_choice: tool_choice.map(value),
        }),
    };
    let counter = recording_counter(Some(9));
    let _total = estimate_precise_input_tokens(
        &translation,
        &FixedResponses {
            messages: Vec::new(),
        },
        &counter,
        &estimate_with(&data, Some("generate_content"), None),
    )
    .await;
    let seen = first_count(&counter);
    assert_eq!(
        seen.tools,
        tools.map(|text| serde_json::from_str::<Vec<Json>>(text).expect("a tool list"))
    );
    assert_eq!(seen.tool_choice, tool_choice.map(value));
}

#[rstest]
#[case::rerank("rerank")]
#[case::text_completion("text_completion")]
#[case::embedding("embedding")]
#[tokio::test]
async fn rerank_text_completion_and_embedding_requests_do_not_forward_messages(
    #[case] call_type: &str,
) {
    let data = request(
        r#"{"messages":[{"role":"user","content":"hi"}],"input":"x","prompt":"p","query":"q"}"#,
    );
    let counter = recording_counter(Some(5));
    let _total = estimate_precise_input_tokens(
        &no_translation(),
        &FixedResponses {
            messages: Vec::new(),
        },
        &counter,
        &estimate_with(&data, Some(call_type), None),
    )
    .await;
    assert_eq!(first_count(&counter).messages, None);
}

#[tokio::test]
async fn without_a_call_type_a_null_prompt_falls_back_to_the_input() {
    let data = request(r#"{"prompt":null,"input":"fallback"}"#);
    let counter = recording_counter(Some(5));
    let _total = estimate_precise_input_tokens(
        &no_translation(),
        &FixedResponses {
            messages: Vec::new(),
        },
        &counter,
        &estimate_with(&data, None, None),
    )
    .await;
    assert_eq!(first_count(&counter).text, Some(value(r#""fallback""#)));
}

#[tokio::test]
async fn a_chat_prompt_wins_over_the_input() {
    let data = request(r#"{"prompt":"first","input":"second"}"#);
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
    assert_eq!(first_count(&counter).text, Some(value(r#""first""#)));
}
