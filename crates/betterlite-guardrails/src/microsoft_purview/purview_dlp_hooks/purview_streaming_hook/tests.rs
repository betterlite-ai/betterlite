#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::microsoft_purview::purview_dlp_hooks::error::PurviewHookError;
use crate::microsoft_purview::purview_dlp_hooks::purview_streaming_hook::{
    ASSEMBLY_ERROR, INCOMPLETE_ERROR, async_post_call_streaming_iterator_hook,
};
use crate::microsoft_purview::purview_dlp_hooks::test_support::{
    Harness, Scan, Script, blocking_actions, data, guardrail_at, hook_context, start,
    trusted_identity, untrusted_identity, values,
};
use crate::purview_dlp_evaluation::error_handling::PurviewDlpHttpError;
use crate::purview_dlp_evaluation::user_id_blocking::UNTRUSTED_IDENTITY_ERROR;
use betterlite_logging::verbose_hooks::{ConsoleSink as _, LoggerSink as _};
use betterlite_redact::structured::Payload;
use betterlite_streaming::stream_usage_assembly::{UsageTokenCounter as _, wire::Usage};
use betterlite_streaming::streamed_response_assembly::cost::{
    CostInput, StreamCostCalculator as _,
};
use rust_decimal::Decimal;

use super::services::{NoCosts, SilentConsole, SilentCounter, SilentLogger};

const CHAT_REQUEST: &str = r#"{"model":"gpt","messages":[{"role":"user","content":"hi"}]}"#;
const CHAT_CHUNKS: &str = r#"[
    {"id":"c1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"role":"assistant","content":"hel"}}]},
    {"id":"c1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"content":"lo"},"finish_reason":"stop"}]}
]"#;
const RESPONSES_DELTA: &str = r#"[
    {"type":"response.output_text.delta","item_id":"i","output_index":0,"content_index":0,"delta":"hel"}
]"#;
const RESPONSES_COMPLETED: &str = r#"[
    {"type":"response.output_text.delta","item_id":"i","output_index":0,"content_index":0,"delta":"hel"},
    {"type":"response.completed","response":{"id":"r1","created_at":1,"object":"response","output":[{"type":"message","id":"m","status":"completed","role":"assistant","content":[{"type":"output_text","text":"hello"}]}]}}
]"#;

fn http_error(result: Result<Vec<GuardrailValue>, PurviewHookError>) -> PurviewDlpHttpError {
    match result {
        Err(PurviewHookError::Http(error)) => *error,
        other => panic!("expected an HTTP error, got {other:?}"),
    }
}

fn detail(error: &PurviewDlpHttpError, key: &str) -> Option<String> {
    match error.detail.get(key) {
        Some(GuardrailValue::Text(text)) => Some(text.clone()),
        _ => None,
    }
}

#[tokio::test]
async fn blocks_before_releasing_any_chunk_the_policy_rejects() {
    let mock = start(Script::new(vec![blocking_actions()]));
    let guardrail = guardrail_at(&mock.base);
    let mut harness = Harness::new(data(CHAT_REQUEST));
    let mut context = hook_context(
        trusted_identity(),
        &harness.cache,
        &harness.redactor,
        &harness.base,
        &harness.sink,
    );
    let result = async_post_call_streaming_iterator_hook(
        &guardrail,
        values(CHAT_CHUNKS),
        &mut harness.data,
        &mut context,
    )
    .await;
    let error = http_error(result);
    assert_eq!(error.status, 400);
    assert_eq!(
        error.detail.get("activity"),
        Some(&GuardrailValue::Text("downloadText".to_owned()))
    );
    assert_eq!(
        mock.scanned(),
        vec![Scan {
            activity: "downloadText".to_owned(),
            data: "hello".to_owned(),
        }]
    );
}

#[tokio::test]
async fn releases_the_original_chunks_after_a_clean_scan() {
    let mock = start(Script::passing());
    let guardrail = guardrail_at(&mock.base);
    let mut harness = Harness::new(data(CHAT_REQUEST));
    let chunks = values(CHAT_CHUNKS);
    let mut context = hook_context(
        trusted_identity(),
        &harness.cache,
        &harness.redactor,
        &harness.base,
        &harness.sink,
    );
    let released = async_post_call_streaming_iterator_hook(
        &guardrail,
        chunks.clone(),
        &mut harness.data,
        &mut context,
    )
    .await
    .unwrap();
    assert_eq!(released, chunks);
    assert_eq!(mock.scanned()[0].data, "hello");
}

#[tokio::test]
async fn rejects_an_incomplete_responses_api_stream() {
    let mock = start(Script::passing());
    let guardrail = guardrail_at(&mock.base);
    let mut harness = Harness::new(data(r#"{"model":"gpt","input":"hi"}"#));
    let mut context = hook_context(
        trusted_identity(),
        &harness.cache,
        &harness.redactor,
        &harness.base,
        &harness.sink,
    );
    let result = async_post_call_streaming_iterator_hook(
        &guardrail,
        values(RESPONSES_DELTA),
        &mut harness.data,
        &mut context,
    )
    .await;
    let error = http_error(result);
    assert_eq!(error.status, 400);
    assert_eq!(detail(&error, "error").as_deref(), Some(INCOMPLETE_ERROR));
    assert_eq!(mock.process_calls(), 0);
}

#[tokio::test]
async fn releases_responses_api_events_after_a_clean_scan() {
    let mock = start(Script::passing());
    let guardrail = guardrail_at(&mock.base);
    let mut harness = Harness::new(data(r#"{"model":"gpt","input":"hi"}"#));
    let chunks = values(RESPONSES_COMPLETED);
    let mut context = hook_context(
        trusted_identity(),
        &harness.cache,
        &harness.redactor,
        &harness.base,
        &harness.sink,
    );
    let released = async_post_call_streaming_iterator_hook(
        &guardrail,
        chunks.clone(),
        &mut harness.data,
        &mut context,
    )
    .await
    .unwrap();
    assert_eq!(released, chunks);
    assert_eq!(
        mock.scanned(),
        vec![Scan {
            activity: "downloadText".to_owned(),
            data: "hello".to_owned(),
        }]
    );
}

#[tokio::test]
async fn fails_closed_when_the_stream_cannot_be_assembled() {
    let mock = start(Script::passing());
    let guardrail = guardrail_at(&mock.base);
    let mut harness = Harness::new(data(CHAT_REQUEST));
    let mut context = hook_context(
        trusted_identity(),
        &harness.cache,
        &harness.redactor,
        &harness.base,
        &harness.sink,
    );
    let result = async_post_call_streaming_iterator_hook(
        &guardrail,
        values(r#"[{"id":"x","choices":[]}]"#),
        &mut harness.data,
        &mut context,
    )
    .await;
    let error = http_error(result);
    assert_eq!(error.status, 400);
    assert_eq!(detail(&error, "error").as_deref(), Some(ASSEMBLY_ERROR));
    assert_eq!(mock.process_calls(), 0);
}

#[tokio::test]
async fn passes_an_empty_stream_without_scanning() {
    let mock = start(Script::passing());
    let guardrail = guardrail_at(&mock.base);
    let mut harness = Harness::new(data(CHAT_REQUEST));
    let mut context = hook_context(
        trusted_identity(),
        &harness.cache,
        &harness.redactor,
        &harness.base,
        &harness.sink,
    );
    let released = async_post_call_streaming_iterator_hook(
        &guardrail,
        Vec::new(),
        &mut harness.data,
        &mut context,
    )
    .await
    .unwrap();
    assert!(released.is_empty());
    assert_eq!(mock.process_calls(), 0);
}

#[tokio::test]
async fn scans_the_assembled_text_of_a_completion_stream() {
    let mock = start(Script::passing());
    let guardrail = guardrail_at(&mock.base);
    let mut harness = Harness::new(data(r#"{"model":"m","prompt":"hi"}"#));
    let chunks = values(
        r#"[
            {"id":"t1","object":"text_completion","created":1,"model":"m","choices":[{"index":0,"text":"hel"}]},
            {"id":"t1","object":"text_completion","created":1,"model":"m","choices":[{"index":0,"text":"lo","finish_reason":"stop"}]}
        ]"#,
    );
    let mut context = hook_context(
        trusted_identity(),
        &harness.cache,
        &harness.redactor,
        &harness.base,
        &harness.sink,
    );
    let released = async_post_call_streaming_iterator_hook(
        &guardrail,
        chunks.clone(),
        &mut harness.data,
        &mut context,
    )
    .await
    .unwrap();
    assert_eq!(released, chunks);
    assert_eq!(
        mock.scanned(),
        vec![Scan {
            activity: "downloadText".to_owned(),
            data: "hello".to_owned(),
        }]
    );
}

#[tokio::test]
async fn seams_stay_silent_and_free() {
    let payload = Payload::Bool(false);
    assert!(!SilentLogger.debug(&payload));
    assert!(!SilentLogger.info(&payload));
    assert!(!SilentLogger.error(&payload));
    assert!(!SilentLogger.debug_enabled());
    assert!(!SilentConsole.print(&payload));
    assert_eq!(
        SilentCounter.count_response_text("m", "t").await.unwrap(),
        0
    );
    assert_eq!(SilentCounter.count_text("t").await.unwrap(), 0);
    assert_eq!(SilentCounter.count_messages("m", None).await.unwrap(), 0);
    let input = CostInput {
        model: "m",
        usage: &Usage::default(),
        hidden_params: &Default::default(),
    };
    assert_eq!(NoCosts.completion_cost(&input, None).unwrap(), None);
    assert_eq!(
        NoCosts
            .prompt_completion_cost("m", &Usage::default())
            .unwrap(),
        (Decimal::ZERO, Decimal::ZERO)
    );
}

#[tokio::test]
async fn rejects_an_identity_the_proxy_did_not_authenticate() {
    let mock = start(Script::passing());
    let guardrail = guardrail_at(&mock.base);
    let mut harness = Harness::new(data(CHAT_REQUEST));
    let mut context = hook_context(
        untrusted_identity(),
        &harness.cache,
        &harness.redactor,
        &harness.base,
        &harness.sink,
    );
    let result = async_post_call_streaming_iterator_hook(
        &guardrail,
        values(CHAT_CHUNKS),
        &mut harness.data,
        &mut context,
    )
    .await;
    let error = http_error(result);
    assert_eq!(
        detail(&error, "error").as_deref(),
        Some(UNTRUSTED_IDENTITY_ERROR)
    );
    assert_eq!(mock.process_calls(), 0);
}
