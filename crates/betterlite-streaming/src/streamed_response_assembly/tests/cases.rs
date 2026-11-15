use betterlite_error::error::ErrorClass;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_tokenizer::count::error::CountError;
use rust_decimal::Decimal;

use crate::stream_chunk_shaping::records::JsonValue;
use crate::stream_content_assembly::wire::{AudioResponse, ExpiryValue, ThinkingBlock};
use crate::streamed_response_assembly::records::Chunk;
use crate::streamed_response_assembly::{AssemblyError, MetadataServices};

use super::support::{
    FixtureLogging, FixtureResolver, FixtureSelector, ProviderSeen, RecordingProjection, chunks,
};
use super::{
    FIXED_NOW, Harness, chat, chat_of, cost_stream, failed, hidden_stream, object, ok,
    plain_stream, text, value,
};

const SOURCE_MESSAGE: &str = "Error building chunks for logging/streaming usage calculation";

pub(super) async fn empty_and_error_boundary() {
    missing_chunks_boundary().await;
    empty_input_boundary().await;
    base_metadata_boundary().await;
    text_error_boundary().await;
}

pub(super) async fn missing_chunks_boundary() {
    let (result, logs) = Harness::new(None).run().await;
    let failure = failed(result);
    assert!(matches!(&failure, AssemblyError::MissingChunks));
    assert_eq!(
        logs,
        vec![
            "litellm.main.py::stream_chunk_builder() - Exception occurred - chunks is required"
                .to_owned()
        ]
    );
    let ErrorClass::Api(details) = failure.classify() else {
        panic!("the builder error must classify as an API error");
    };
    assert_eq!(details.status, 500);
    assert_eq!(details.context.message, SOURCE_MESSAGE);
    assert_eq!(details.context.model.as_deref(), Some(""));
    assert_eq!(details.context.provider.as_deref(), Some(""));
}

pub(super) async fn empty_input_boundary() {
    let (result, logs) = Harness::new(Some(Vec::new())).run().await;
    assert!(matches!(result, Ok(None)));
    assert!(logs.is_empty());
}

pub(super) async fn base_metadata_boundary() {
    let (result, logs) = Harness::new(Some(chunks(r#"[{"choices": []}]"#)))
        .run()
        .await;
    assert!(matches!(failed(result), AssemblyError::Base(_)));
    assert_eq!(logs.len(), 1);
    assert!(logs[0].ends_with("Exception occurred - the first chunk has no object"));
    let source = r#"[{"object": "chat.completion.chunk", "model": "gpt-4o", "choices": []}]"#;
    let (result, logs) = Harness::new(Some(chunks(source))).run().await;
    assert!(matches!(failed(result), AssemblyError::Base(_)));
    assert!(logs[0].ends_with("Exception occurred - the first chunk has no created timestamp"));
}

pub(super) async fn text_error_boundary() {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "text_completion", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "text": "Hi"}]},
        {"choices": []}
    ]"#,
    );
    let (result, logs) = Harness::new(Some(stream)).run().await;
    assert!(matches!(failed(result), AssemblyError::MissingLastChoice));
    assert_eq!(logs.len(), 1);
    assert!(logs[0].ends_with("Exception occurred - the last chunk has no first choice"));
}

pub(super) async fn text_completion_assembly() {
    text_completion_join().await;
    text_defaults().await;
    text_completion_failure().await;
    text_total_overflow().await;
    text_branch_selection().await;
}

pub(super) async fn text_completion_join() {
    let logprobs = value(
        r#"{"tokens": ["Hel"], "token_logprobs": [-0.1], "top_logprobs": [{}], "text_offset": [0]}"#,
    );
    let stream = chunks(
        r#"[{"id": "chatcmpl-text", "object": "text_completion", "created": 1700000000, "model": "gpt-4o",
         "system_fingerprint": "fp_1", "choices": [{"index": 0, "text": "Hel"}]},
        {"choices": [{"index": 0, "text": "lo", "finish_reason": "length"}, {"index": 1, "text": "!"}]},
        {"choices": [{"index": 0, "text": null, "finish_reason": "stop",
         "logprobs": {"tokens": ["Hel"], "token_logprobs": [-0.1], "top_logprobs": [{}], "text_offset": [0]}}]}]"#,
    );
    let mut harness = Harness::new(Some(stream));
    harness.messages = true;
    harness.prompt = Some(Box::new(|| Ok(7)));
    harness.counter.messages = 3;
    harness.counter.response_text = 2;
    let (result, logs) = harness.run().await;
    let response = text(ok(result));
    assert_eq!(response.id, "chatcmpl-text");
    assert_eq!(response.object, "text_completion");
    assert_eq!(response.created, FIXED_NOW);
    assert_eq!(response.model.as_deref(), Some("gpt-4o"));
    assert_eq!(response.system_fingerprint.as_deref(), Some("fp_1"));
    assert_eq!(response.choices.len(), 1);
    assert_eq!(response.choices[0].text, "Hello!");
    assert_eq!(response.choices[0].index, 0);
    assert_eq!(response.choices[0].finish_reason.as_deref(), Some("stop"));
    assert_eq!(response.choices[0].logprobs, Some(logprobs));
    assert_eq!(response.usage.prompt_tokens, 3);
    assert_eq!(response.usage.completion_tokens, 2);
    assert_eq!(response.usage.total_tokens, 5);
    assert!(logs.is_empty());
}

pub(super) async fn text_defaults() {
    let stream = chunks(r#"[{"choices": [{"index": 0, "text": "Solo"}]}]"#);
    let mut harness = Harness::new(Some(stream));
    harness.counter.response_text = 1;
    let (result, logs) = harness.run().await;
    let response = text(ok(result));
    assert!(response.id.starts_with("chatcmpl-"));
    assert_eq!(response.object, "text_completion");
    assert_eq!(response.created, FIXED_NOW);
    assert_eq!(response.model, None);
    assert_eq!(response.system_fingerprint, None);
    assert_eq!(response.choices[0].text, "Solo");
    assert_eq!(response.choices[0].finish_reason, None);
    assert_eq!(response.choices[0].logprobs, None);
    assert_eq!(response.usage.prompt_tokens, 0);
    assert_eq!(response.usage.completion_tokens, 1);
    assert_eq!(response.usage.total_tokens, 1);
    assert_eq!(
        logs,
        vec!["token_counter failed, assuming prompt tokens is 0".to_owned()]
    );
}

pub(super) async fn text_completion_failure() {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "text_completion", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "text": "A"}]}
    ]"#,
    );
    let mut harness = Harness::new(Some(stream));
    harness.counter.fail_response_text = true;
    let (result, logs) = harness.run().await;
    assert!(matches!(failed(result), AssemblyError::TokenCount { .. }));
    assert_eq!(logs.len(), 2);
    assert_eq!(logs[0], "token_counter failed, assuming prompt tokens is 0");
    assert!(logs[1].ends_with(
        "Exception occurred - the token count failed: either text or messages must be provided"
    ));
}

pub(super) async fn text_total_overflow() {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "text_completion", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "text": "A"}]}
    ]"#,
    );
    let mut harness = Harness::new(Some(stream));
    harness.messages = true;
    harness.counter.messages = i64::MAX;
    harness.counter.response_text = 1;
    let (result, logs) = harness.run().await;
    assert!(matches!(failed(result), AssemblyError::Overflow));
    assert!(logs[0].ends_with("Exception occurred - the token count overflowed"));
}

pub(super) async fn text_branch_selection() {
    let lead = chunks(
        r#"[
        {"id": "chatcmpl-lead", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": []},
        {"choices": [{"index": 0, "text": "AB"}]}
    ]"#,
    );
    let mut harness = Harness::new(Some(lead));
    harness.messages = true;
    harness.counter.messages = 1;
    harness.counter.response_text = 1;
    let (result, _logs) = harness.run().await;
    let response = text(ok(result));
    assert_eq!(response.id, "chatcmpl-lead");
    assert_eq!(response.object, "text_completion");
    assert_eq!(response.choices[0].text, "AB");
    let mixed = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"content": "A"}}]},
        {"choices": [{"index": 0, "text": "B"}]}
    ]"#,
    );
    let mut harness = Harness::new(Some(mixed));
    harness.messages = true;
    harness.counter.response_text = 1;
    let response = chat_of(&harness).await;
    assert_eq!(response.object, "chat.completion");
    assert_eq!(response.choices[0].message.content.as_deref(), Some("A"));
}

pub(super) async fn simple_text_path() {
    simple_text_join().await;
    simple_unsupported_deltas().await;
    simple_first_choice_content().await;
    simple_disabled_by_fields().await;
    simple_any_choice_disables().await;
    simple_no_parts().await;
}

pub(super) async fn simple_text_join() {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"role": "assistant", "content": "A"}}]},
        {"choices": []},
        {"choices": [{"index": 0, "delta": {"content": "B"}, "finish_reason": "stop"}]}
    ]"#,
    );
    let mut harness = Harness::new(Some(stream));
    harness.messages = true;
    harness.counter.messages = 5;
    harness.counter.response_text = 3;
    let response = chat_of(&harness).await;
    assert_eq!(response.choices[0].message.content.as_deref(), Some("AB"));
    assert_eq!(response.choices[0].message.role, "assistant");
    assert_eq!(response.choices[0].finish_reason, "stop");
    assert_eq!(response.usage.prompt_tokens, 5);
    assert_eq!(response.usage.completion_tokens, 3);
    assert_eq!(response.usage.total_tokens, 8);
    let details = response
        .usage
        .completion_tokens_details
        .expect("the simple path stamps zero reasoning tokens");
    assert_eq!(details.reasoning_tokens, Some(0));
    assert_eq!(details.text_tokens, Some(3));
}

pub(super) async fn simple_unsupported_deltas() {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"content": "A"}}]},
        {"choices": [{"index": 0, "delta": null}]},
        {"choices": [{"index": 0, "delta": "text"}]},
        {"choices": [{"index": 0, "delta": [1, 2]}]}
    ]"#,
    );
    let mut harness = Harness::new(Some(stream));
    harness.messages = true;
    harness.counter.response_text = 1;
    let response = chat_of(&harness).await;
    assert_eq!(response.choices[0].message.content.as_deref(), Some("A"));
    let details = response
        .usage
        .completion_tokens_details
        .expect("unsupported deltas keep the simple path");
    assert_eq!(details.reasoning_tokens, Some(0));
}

pub(super) async fn simple_first_choice_content() {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"content": "A"}}]},
        {"choices": [{"index": 0, "delta": {"role": "assistant"}}, {"index": 1, "delta": {"content": "Z"}}]},
        {"choices": [{"index": 0, "delta": {"content": "B"}}]}
    ]"#,
    );
    let mut harness = Harness::new(Some(stream));
    harness.messages = true;
    harness.counter.response_text = 2;
    let response = chat_of(&harness).await;
    assert_eq!(response.choices[0].message.content.as_deref(), Some("AB"));
}

pub(super) async fn simple_disabled_by_fields() {
    let disabling = [
        ("tool_calls", value("[]")),
        ("function_call", value(r#"{"name": "f", "arguments": ""}"#)),
        ("reasoning_content", value(r#""""#)),
        ("thinking_blocks", value("[]")),
        ("annotations", value("[]")),
        ("audio", value("{}")),
        ("images", value("[]")),
        ("provider_specific_fields", value("{}")),
    ];
    for (field, disabled) in disabling {
        let mut delta = object(r#"{"content": "B"}"#);
        delta.insert(field.to_owned(), disabled);
        let mut source = String::from(
            r#"[{"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o", "choices": [{"index": 0, "delta": {"role": "assistant", "content": "A"}}]}, {"choices": [{"index": 0, "delta": "#,
        );
        source.push_str(&serde_json::to_string(&delta).expect("fixture delta serializes"));
        source.push_str("}]}]");
        let stream = chunks(&source);
        let mut harness = Harness::new(Some(stream));
        harness.messages = true;
        harness.counter.response_text = 2;
        let response = chat_of(&harness).await;
        assert_eq!(
            response.choices[0].message.content.as_deref(),
            Some("AB"),
            "{field}"
        );
        assert_eq!(
            response.usage.completion_tokens_details, None,
            "{field} must leave the fast path"
        );
    }
}

pub(super) async fn simple_any_choice_disables() {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"role": "assistant", "content": "A"}}]},
        {"choices": [
            {"index": 0, "delta": {"content": "B"}},
            {"index": 1, "delta": {"tool_calls": []}}
        ]}
    ]"#,
    );
    let mut harness = Harness::new(Some(stream));
    harness.messages = true;
    harness.counter.response_text = 2;
    let response = chat_of(&harness).await;
    assert_eq!(response.choices[0].message.content.as_deref(), Some("AB"));
    assert_eq!(response.usage.completion_tokens_details, None);
}

pub(super) async fn simple_no_parts() {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"role": "assistant"}}]},
        {"choices": [{"index": 0, "delta": {}}]}
    ]"#,
    );
    let mut harness = Harness::new(Some(stream));
    harness.messages = true;
    harness.counter.response_text = 0;
    let response = chat_of(&harness).await;
    assert_eq!(response.choices[0].message.content.as_deref(), Some(""));
    assert_eq!(response.choices[0].message.role, "assistant");
    let details = response
        .usage
        .completion_tokens_details
        .expect("the simple path still runs without text parts");
    assert_eq!(details.reasoning_tokens, Some(0));
}

pub(super) async fn complex_content_order() {
    complex_tool_calls().await;
    complex_function_call().await;
    complex_lists().await;
    complex_thinking().await;
    complex_audio().await;
    complex_audio_default_expiry().await;
}

pub(super) async fn complex_tool_calls() {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"role": "assistant", "content": "Hi"}}]},
        {"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "id": "call_1", "type": "function",
            "function": {"name": "get_weather", "arguments": ""}}]}}]},
        {"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0,
            "function": {"arguments": "{\"city\":\"Paris\"}"}}]}}]},
        {"choices": [{"index": 0, "delta": {"content": " there"}, "finish_reason": "stop"}],
         "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}}
    ]"#,
    );
    let response = chat_of(&Harness::new(Some(stream))).await;
    let message = &response.choices[0].message;
    assert_eq!(message.content.as_deref(), Some("Hi there"));
    let calls = message
        .tool_calls
        .as_ref()
        .expect("the tool call fragments are assembled");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "call_1");
    assert_eq!(calls[0].kind, "function");
    let function = calls[0]
        .function
        .as_ref()
        .expect("the assembled call carries its function");
    assert_eq!(function.name.as_deref(), Some("get_weather"));
    assert_eq!(function.arguments, r#"{"city":"Paris"}"#);
    assert!(message.function_call.is_none());
    assert_eq!(response.usage.completion_tokens, 5);
    assert_eq!(response.usage.completion_tokens_details, None);
}

pub(super) async fn complex_function_call() {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"role": "assistant", "content": "Hi"}}]},
        {"choices": [{"index": 0, "delta": {"function_call": {"name": "get_weather",
            "arguments": "{\"city\":\""}}}]},
        {"choices": [{"index": 0, "delta": {"function_call": {"arguments": "Paris\"}"}}}]},
        {"choices": [{"index": 0, "delta": {"content": " there"}}]}
    ]"#,
    );
    let mut harness = Harness::new(Some(stream));
    harness.messages = true;
    harness.counter.response_text = 2;
    let response = chat_of(&harness).await;
    let message = &response.choices[0].message;
    assert_eq!(message.content.as_deref(), Some("Hi there"));
    assert!(message.tool_calls.is_none());
    let function = message
        .function_call
        .as_ref()
        .expect("the function call fragments are assembled");
    assert_eq!(function.name.as_deref(), Some("get_weather"));
    assert_eq!(function.arguments, r#"{"city":"Paris"}"#);
}

pub(super) async fn complex_lists() {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"content": "A",
            "annotations": [{"url": "u1"}], "images": [{"url": "img1"}]}}]},
        {"choices": [{"index": 0, "delta": {"annotations": [{"url": "u2"}], "images": []}}]}
    ]"#,
    );
    let mut harness = Harness::new(Some(stream));
    harness.messages = true;
    harness.counter.response_text = 1;
    let response = chat_of(&harness).await;
    let message = &response.choices[0].message;
    assert_eq!(
        message.annotations,
        Some(vec![value(r#"{"url": "u1"}"#), value(r#"{"url": "u2"}"#)])
    );
    assert_eq!(message.images, Some(vec![value(r#"{"url": "img1"}"#)]));
}

pub(super) async fn complex_thinking() {
    let mut harness = Harness::new(Some(thinking_stream()));
    harness.messages = true;
    harness.counter.text = 2;
    harness.counter.response_text = 3;
    let response = chat_of(&harness).await;
    let message = &response.choices[0].message;
    assert_eq!(message.reasoning_content.as_deref(), Some("thinking"));
    assert_eq!(
        message.thinking_blocks,
        Some(vec![
            ThinkingBlock::Thinking {
                thinking: "deep".to_owned(),
                signature: "sig".to_owned(),
            },
            ThinkingBlock::Redacted {
                data: "d1".to_owned(),
            },
        ])
    );
    assert_eq!(
        harness.counter.seen_reasoning().as_deref(),
        Some("thinking")
    );
    assert_eq!(response.usage.completion_tokens, 3);
    let details = response
        .usage
        .completion_tokens_details
        .expect("the complex path counts reasoning tokens");
    assert_eq!(details.reasoning_tokens, Some(2));
    assert_eq!(details.text_tokens, Some(1));
}

fn thinking_stream() -> Vec<Chunk> {
    chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"role": "assistant",
            "thinking_blocks": [{"type": "thinking", "thinking": "deep", "signature": "sig"}],
            "reasoning_content": "think"}}]},
        {"choices": [{"index": 0, "delta": {
            "thinking_blocks": [{"type": "redacted_thinking", "data": "d1"}],
            "reasoning_content": "ing"}}]}
    ]"#,
    )
}

pub(super) async fn complex_audio() {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"content": "A",
            "audio": {"id": "a1", "data": "YQ==", "transcript": "hi"}}}]},
        {"choices": [{"index": 0, "delta": {
            "audio": {"data": "Yg==", "transcript": "!", "expires_at": 1600000000}}}]}
    ]"#,
    );
    let mut harness = Harness::new(Some(stream));
    harness.messages = true;
    harness.counter.response_text = 1;
    let response = chat_of(&harness).await;
    assert_eq!(
        response.choices[0].message.audio,
        Some(AudioResponse {
            data: "YWI=".to_owned(),
            expires_at: ExpiryValue::Signed(1_600_000_000),
            transcript: "hi!".to_owned(),
            id: Some("a1".to_owned()),
        })
    );
}

pub(super) async fn complex_audio_default_expiry() {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"content": "A",
            "audio": {"data": "YQ==", "transcript": "hi", "expires_at": false}}}]}
    ]"#,
    );
    let mut harness = Harness::new(Some(stream));
    harness.messages = true;
    harness.counter.response_text = 1;
    let response = chat_of(&harness).await;
    let audio = response.choices[0]
        .message
        .audio
        .as_ref()
        .expect("the audio fragments are assembled");
    assert_eq!(audio.expires_at, ExpiryValue::Signed(FIXED_NOW + 3600));
    assert_eq!(audio.data, "YQ==");
}

pub(super) async fn provider_fields_citations() {
    provider_fields_merge().await;
    provider_fields_non_list_citations().await;
    provider_fields_explicit_override().await;
    provider_fields_skip_non_dict().await;
    provider_fields_empty_map().await;
    provider_fields_null_citation().await;
}

pub(super) async fn provider_fields_merge() {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"content": "A",
            "provider_specific_fields": {"citation": ["c1", "c2"]}}}]},
        {"choices": [{"index": 0, "delta": {"provider_specific_fields":
            {"citation": ["c3"], "web_search_results": ["t1"]}}}]},
        {"choices": [{"index": 0, "delta": {"provider_specific_fields":
            {"web_search_results": ["t2"], "citation_count": 3}}}]}
    ]"#,
    );
    let mut harness = Harness::new(Some(stream));
    harness.messages = true;
    harness.counter.response_text = 1;
    let response = chat_of(&harness).await;
    let expected = object(
        r#"{"citations": [["c1", "c2"], ["c3"]], "web_search_results": ["t2"], "citation_count": 3}"#,
    );
    assert_eq!(
        response.choices[0].message.provider_specific_fields,
        Some(expected)
    );
}

pub(super) async fn provider_fields_non_list_citations() {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"content": "A",
            "provider_specific_fields": {"citation": "c1"}}}]},
        {"choices": [{"index": 0, "delta": {"provider_specific_fields": {"citation": "c2"}}}]}
    ]"#,
    );
    let mut harness = Harness::new(Some(stream));
    harness.messages = true;
    harness.counter.response_text = 1;
    let response = chat_of(&harness).await;
    let expected = object(r#"{"citations": [["c1", "c2"]]}"#);
    assert_eq!(
        response.choices[0].message.provider_specific_fields,
        Some(expected)
    );
}

pub(super) async fn provider_fields_explicit_override() {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"content": "A",
            "provider_specific_fields": {"citation": ["g1"]}}}]},
        {"choices": [{"index": 0, "delta": {"provider_specific_fields": {"citations": ["mine"]}}}]}
    ]"#,
    );
    let mut harness = Harness::new(Some(stream));
    harness.messages = true;
    harness.counter.response_text = 1;
    let response = chat_of(&harness).await;
    let expected = object(r#"{"citations": ["mine"]}"#);
    assert_eq!(
        response.choices[0].message.provider_specific_fields,
        Some(expected)
    );
}

pub(super) async fn provider_fields_skip_non_dict() {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"content": "A", "provider_specific_fields": "nope"}}]},
        {"choices": [{"index": 0, "delta": {"provider_specific_fields": {"k": 1}}}]}
    ]"#,
    );
    let mut harness = Harness::new(Some(stream));
    harness.messages = true;
    harness.counter.response_text = 1;
    let response = chat_of(&harness).await;
    let expected = object(r#"{"k": 1}"#);
    assert_eq!(
        response.choices[0].message.provider_specific_fields,
        Some(expected)
    );
}

pub(super) async fn provider_fields_empty_map() {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"content": "A", "provider_specific_fields": {}}}]}
    ]"#,
    );
    let mut harness = Harness::new(Some(stream));
    harness.messages = true;
    harness.counter.response_text = 1;
    let response = chat_of(&harness).await;
    assert_eq!(response.choices[0].message.provider_specific_fields, None);
}

pub(super) async fn provider_fields_null_citation() {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"content": "A",
            "provider_specific_fields": {"citation": null, "warning": "w"}}}]},
        {"choices": [{"index": 0, "delta": {"provider_specific_fields":
            {"citation": ["c9"], "web_search_results": ["t1"]}}}]}
    ]"#,
    );
    let mut harness = Harness::new(Some(stream));
    harness.messages = true;
    harness.counter.response_text = 1;
    let response = chat_of(&harness).await;
    let expected =
        object(r#"{"citations": [["c9"]], "warning": "w", "web_search_results": ["t1"]}"#);
    assert_eq!(
        response.choices[0].message.provider_specific_fields,
        Some(expected)
    );
}

pub(super) async fn usage_hidden_metadata() {
    usage_reasoning_and_hidden().await;
    hidden_provider_fields_scan().await;
    hidden_provider_fields_stop().await;
    hidden_provider_fields_error().await;
    hidden_non_object_skipped().await;
    prompt_callback_failure().await;
    metadata_after_cost().await;
}

pub(super) async fn usage_reasoning_and_hidden() {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"role": "assistant", "content": "Hello",
            "reasoning_content": "r"}}]},
        {"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
         "_hidden_params": {"provider_specific_fields": {"traffic_type": "on_demand"}}}
    ]"#,
    );
    let mut harness = Harness::new(Some(stream));
    harness.prompt = Some(Box::new(|| Ok(10)));
    harness.counter.text = 1;
    harness.counter.response_text = 2;
    let response = chat_of(&harness).await;
    assert_eq!(harness.counter.seen_completion().as_deref(), Some("Hello"));
    assert_eq!(harness.counter.seen_reasoning().as_deref(), Some("r"));
    assert_eq!(harness.counter.seen_model().as_deref(), Some("gpt-4o"));
    assert_eq!(response.usage.prompt_tokens, 10);
    assert_eq!(response.usage.completion_tokens, 2);
    assert_eq!(response.usage.total_tokens, 12);
    let details = response
        .usage
        .completion_tokens_details
        .as_ref()
        .expect("the complex path counts reasoning tokens");
    assert_eq!(details.reasoning_tokens, Some(1));
    assert_eq!(details.text_tokens, Some(1));
    assert_eq!(
        response.hidden_params.get("provider_specific_fields"),
        Some(&value(r#"{"traffic_type": "on_demand"}"#))
    );
}

pub(super) async fn hidden_provider_fields_scan() {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"content": "A"}}],
         "_hidden_params": {"provider_specific_fields": {"a": 1}}},
        {"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
         "_hidden_params": {"other": "x"},
         "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}}
    ]"#,
    );
    let response = chat_of(&Harness::new(Some(stream))).await;
    assert_eq!(
        response.hidden_params,
        object(r#"{"other": "x", "provider_specific_fields": {"a": 1}}"#)
    );
}

pub(super) async fn hidden_provider_fields_stop() {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"content": "A"}}],
         "_hidden_params": {"provider_specific_fields": {"a": 1}}},
        {"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
         "_hidden_params": {"provider_specific_fields": {}},
         "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}}
    ]"#,
    );
    let response = chat_of(&Harness::new(Some(stream))).await;
    assert_eq!(
        response.hidden_params,
        object(r#"{"provider_specific_fields": {}}"#)
    );
}

pub(super) async fn hidden_provider_fields_error() {
    let stream = hidden_stream(r#"{"provider_specific_fields": "nope"}"#);
    let (result, logs) = Harness::new(Some(stream)).run().await;
    assert!(matches!(
        failed(result),
        AssemblyError::InvalidProviderFields
    ));
    assert!(
        logs[0].ends_with("Exception occurred - provider_specific_fields is not a JSON object")
    );
}

pub(super) async fn hidden_non_object_skipped() {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"content": "A"}}],
         "_hidden_params": {"provider_specific_fields": {"a": 1}}},
        {"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
         "_hidden_params": "nope",
         "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}}
    ]"#,
    );
    let response = chat_of(&Harness::new(Some(stream))).await;
    assert_eq!(
        response.hidden_params,
        object(r#"{"provider_specific_fields": {"a": 1}}"#)
    );
}

pub(super) async fn prompt_callback_failure() {
    let mut harness = Harness::new(Some(plain_stream()));
    harness.messages = true;
    harness.prompt = Some(Box::new(|| Err(CountError::MissingInput)));
    harness.counter.response_text = 1;
    let (result, logs) = harness.run().await;
    let response = chat(ok(result));
    assert_eq!(response.usage.prompt_tokens, 0);
    assert_eq!(response.usage.completion_tokens, 1);
    assert!(
        logs.iter()
            .any(|line| line == "token_counter failed, assuming prompt tokens is 0")
    );
}

pub(super) async fn metadata_after_cost() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("fixture redactor compiles");
    let resolver = FixtureResolver;
    let selector = FixtureSelector;
    let projection = RecordingProjection::default();
    let services = MetadataServices {
        redactor: &redactor,
        resolver: &resolver,
        selector: &selector,
        projection: Some(&projection),
    };
    let mut logging = FixtureLogging::default();
    logging.provider = Some("openai");
    logging.cost = Some(Decimal::new(75, 6));
    let mut harness = Harness::new(Some(cost_stream(None)));
    harness.logging = Some(&logging);
    harness.metadata = Some(&services);
    let response = chat_of(&harness).await;
    assert_eq!(projection.seen_cost(), Some(value("0.000075")));
    assert_eq!(
        response.hidden_params.get("response_cost"),
        Some(&value("0.000075"))
    );
    assert_eq!(
        response.hidden_params.get("projected"),
        Some(&JsonValue::String("OpenAIGPTConfig".to_owned()))
    );
    assert_eq!(response.usage.cost, Some(Decimal::new(75, 6)));
}

pub(super) async fn reported_cost_precedence() {
    reported_cost_reported().await;
    reported_cost_bool().await;
    reported_cost_xai().await;
    reported_cost_logging_without_reported().await;
    reported_cost_provider_hint().await;
    reported_cost_fallback().await;
    reported_cost_fallback_failure().await;
    reported_cost_preserved().await;
}

pub(super) async fn reported_cost_reported() {
    let harness = Harness::new(Some(cost_stream(Some("0.25"))));
    let response = chat_of(&harness).await;
    assert_eq!(response.usage.cost, Some(Decimal::new(25, 2)));
    assert_eq!(
        response.hidden_params.get("response_cost"),
        Some(&value("0.25"))
    );
    assert_eq!(harness.costs.completion_calls(), 0);
}

pub(super) async fn reported_cost_bool() {
    let harness = Harness::new(Some(cost_stream(Some("true"))));
    let response = chat_of(&harness).await;
    assert_eq!(response.usage.cost, Some(Decimal::ONE));
    assert_eq!(
        response.hidden_params.get("response_cost"),
        Some(&value("1"))
    );
}

pub(super) async fn reported_cost_xai() {
    let mut logging = FixtureLogging::default();
    logging.provider = Some("xai");
    let mut harness = Harness::new(Some(cost_stream(Some("0.25"))));
    harness.logging = Some(&logging);
    let response = chat_of(&harness).await;
    assert_eq!(response.usage.cost, Some(Decimal::new(25, 2)));
    assert_eq!(response.hidden_params.get("response_cost"), None);
    assert_eq!(logging.calls(), 0);
    assert_eq!(harness.costs.completion_calls(), 0);
}

pub(super) async fn reported_cost_logging_without_reported() {
    let mut logging = FixtureLogging::default();
    logging.provider = Some("openai");
    let mut harness = Harness::new(Some(plain_stream()));
    harness.messages = true;
    harness.counter.response_text = 1;
    harness.logging = Some(&logging);
    let response = chat_of(&harness).await;
    assert_eq!(response.hidden_params.get("response_cost"), None);
    assert_eq!(harness.costs.completion_calls(), 0);
    assert_eq!(logging.calls(), 1);
}

pub(super) async fn reported_cost_provider_hint() {
    let mut harness = Harness::new(Some(hidden_stream(r#"{"custom_llm_provider": "openai"}"#)));
    harness.costs.completion = Some(Decimal::new(5, 1));
    let response = chat_of(&harness).await;
    assert_eq!(
        harness.costs.seen_provider(),
        ProviderSeen::Named(Some("openai".to_owned()))
    );
    assert_eq!(
        response.hidden_params.get("response_cost"),
        Some(&value("0.5"))
    );
}

pub(super) async fn reported_cost_fallback() {
    let mut harness = Harness::new(Some(plain_stream()));
    harness.messages = true;
    harness.counter.response_text = 1;
    harness.costs.completion_error = true;
    harness.costs.prompt = Some(Decimal::new(1, 4));
    harness.costs.completion_tokens = Some(Decimal::new(2, 4));
    let response = chat_of(&harness).await;
    assert_eq!(harness.costs.completion_calls(), 1);
    assert_eq!(
        response.hidden_params.get("response_cost"),
        Some(&value("0.0003"))
    );
}

pub(super) async fn reported_cost_fallback_failure() {
    let mut harness = Harness::new(Some(plain_stream()));
    harness.messages = true;
    harness.counter.response_text = 1;
    harness.costs.completion_error = true;
    harness.costs.per_token_error = true;
    let response = chat_of(&harness).await;
    assert_eq!(response.hidden_params.get("response_cost"), None);
    let empty_model = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "",
         "choices": [{"index": 0, "delta": {"content": "A"}}]}
    ]"#,
    );
    let mut harness = Harness::new(Some(empty_model));
    harness.messages = true;
    harness.counter.response_text = 1;
    harness.costs.completion_error = true;
    harness.costs.prompt = Some(Decimal::new(1, 4));
    harness.costs.completion_tokens = Some(Decimal::new(2, 4));
    let response = chat_of(&harness).await;
    assert_eq!(response.hidden_params.get("response_cost"), None);
}

pub(super) async fn reported_cost_preserved() {
    let harness = Harness::new(Some(hidden_stream(r#"{"response_cost": 0.5}"#)));
    let response = chat_of(&harness).await;
    assert_eq!(
        response.hidden_params.get("response_cost"),
        Some(&value("0.5"))
    );
}

pub(super) async fn usage_cost_stamping() {
    stamping_from_logging().await;
    stamping_requires_logging().await;
    stamping_skips_existing().await;
    stamping_zero_calculator().await;
    stamping_still_applies_to_xai().await;
    stamping_error().await;
}

pub(super) async fn stamping_from_logging() {
    let mut logging = FixtureLogging::default();
    logging.provider = Some("openai");
    logging.cost = Some(Decimal::new(75, 6));
    let mut harness = Harness::new(Some(cost_stream(None)));
    harness.logging = Some(&logging);
    let response = chat_of(&harness).await;
    assert_eq!(response.usage.cost, Some(Decimal::new(75, 6)));
    assert_eq!(
        response.hidden_params.get("response_cost"),
        Some(&value("0.000075"))
    );
    assert_eq!(logging.calls(), 1);
}

pub(super) async fn stamping_requires_logging() {
    let harness = Harness::new(Some(cost_stream(None)));
    let response = chat_of(&harness).await;
    assert_eq!(response.usage.cost, None);
    assert_eq!(response.hidden_params.get("response_cost"), None);
}

pub(super) async fn stamping_skips_existing() {
    let existing = [
        ("0", Decimal::ZERO),
        ("-0.5", Decimal::new(-5, 1)),
        ("true", Decimal::ONE),
    ];
    for (reported, expected) in existing {
        let mut logging = FixtureLogging::default();
        logging.provider = Some("openai");
        logging.cost = Some(Decimal::new(75, 6));
        let mut harness = Harness::new(Some(cost_stream(Some(reported))));
        harness.logging = Some(&logging);
        let response = chat_of(&harness).await;
        assert_eq!(response.usage.cost, Some(expected));
        assert_eq!(
            response.hidden_params.get("response_cost"),
            Some(&value(&expected.to_string()))
        );
        assert_eq!(logging.calls(), 0);
    }
}

pub(super) async fn stamping_zero_calculator() {
    let mut logging = FixtureLogging::default();
    logging.provider = Some("openai");
    logging.cost = Some(Decimal::ZERO);
    let mut harness = Harness::new(Some(cost_stream(None)));
    harness.logging = Some(&logging);
    let response = chat_of(&harness).await;
    assert_eq!(response.usage.cost, None);
    assert_eq!(response.hidden_params.get("response_cost"), None);
    assert_eq!(logging.calls(), 1);
}

pub(super) async fn stamping_still_applies_to_xai() {
    let mut logging = FixtureLogging::default();
    logging.provider = Some("xai");
    logging.cost = Some(Decimal::new(75, 6));
    let mut harness = Harness::new(Some(cost_stream(None)));
    harness.logging = Some(&logging);
    let response = chat_of(&harness).await;
    assert_eq!(response.usage.cost, Some(Decimal::new(75, 6)));
    assert_eq!(response.hidden_params.get("response_cost"), None);
    assert_eq!(logging.calls(), 1);
}

pub(super) async fn stamping_error() {
    let mut logging = FixtureLogging::default();
    logging.provider = Some("openai");
    logging.fail = true;
    let mut harness = Harness::new(Some(cost_stream(None)));
    harness.logging = Some(&logging);
    let (result, logs) = harness.run().await;
    assert!(matches!(failed(result), AssemblyError::Cost(_)));
    assert!(logs[0].ends_with(
        "Exception occurred - the cost calculation failed: fixture logging cost failure"
    ));
}
