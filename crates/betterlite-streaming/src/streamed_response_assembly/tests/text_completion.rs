use std::sync::Arc;

use betterlite_redact::secret::Redactor;

use crate::streamed_response_assembly::AssemblyError;
use crate::streamed_response_assembly::response::TextResponse;

use super::support::chunks;
use super::{FIXED_NOW, Harness, chat_of, failed, ok, text, value};

pub(super) async fn text_completion_assembly(redactor: &Arc<Redactor>) {
    text_completion_join(redactor).await;
    text_completion_usage(redactor).await;
    text_defaults(redactor).await;
    text_completion_failure(redactor).await;
    text_total_overflow(redactor).await;
    text_branch_selection(redactor).await;
    text_branch_mixed(redactor).await;
}

const JOIN_STREAM: &str = r#"[
    {"id": "chatcmpl-text", "object": "text_completion", "created": 1700000000, "model": "gpt-4o",
     "system_fingerprint": "fp_1", "choices": [{"index": 0, "text": "Hey"}]},
    {"created": 1700000500, "choices": [{"index": 0, "text": " you", "finish_reason": "length"}, {"index": 1, "text": "!"}]},
    {"created": 1700000999, "choices": [{"index": 0, "text": null, "finish_reason": "stop",
     "logprobs": {"tokens": ["Hey"], "token_logprobs": [-0.1], "top_logprobs": [{}], "text_offset": [0]}}]}
]"#;

async fn join_run(redactor: &Arc<Redactor>) -> (TextResponse, Vec<String>) {
    let mut harness = Harness::new(Some(chunks(JOIN_STREAM)), redactor);
    harness.messages = true;
    harness.prompt = Some(Box::new(|| Ok(7)));
    harness.counter.messages = 3;
    harness.counter.response_text = 2;
    harness.now = 1_800_000_000;
    let (result, logs) = harness.run().await;
    (text(ok(result)), logs)
}

pub(super) async fn text_completion_join(redactor: &Arc<Redactor>) {
    let logprobs = value(
        r#"{"tokens": ["Hey"], "token_logprobs": [-0.1], "top_logprobs": [{}], "text_offset": [0]}"#,
    );
    let (response, logs) = join_run(redactor).await;
    let metadata = (
        response.id.as_str(),
        response.object.as_str(),
        response.created,
        response.model.as_deref(),
        response.system_fingerprint.as_deref(),
    );
    assert_eq!(
        metadata,
        (
            "chatcmpl-text",
            "text_completion",
            1_700_000_000,
            Some("gpt-4o"),
            Some("fp_1")
        )
    );
    let choice = &response.choices[0];
    assert_eq!(response.choices.len(), 1);
    assert_eq!(
        (
            choice.text.as_str(),
            choice.index,
            choice.finish_reason.as_deref()
        ),
        ("Hey you!", 0, Some("stop"))
    );
    assert_eq!(choice.logprobs, Some(logprobs));
    assert!(logs.is_empty());
}

// The prompt count comes from the message counter, since the text builder never sees the callback.
pub(super) async fn text_completion_usage(redactor: &Arc<Redactor>) {
    let (response, _logs) = join_run(redactor).await;
    let usage = &response.usage;
    assert_eq!(
        (
            usage.prompt_tokens,
            usage.completion_tokens,
            usage.total_tokens
        ),
        (3, 2, 5)
    );
}

pub(super) async fn text_defaults(redactor: &Arc<Redactor>) {
    let stream = chunks(r#"[{"choices": [{"index": 0, "text": "Solo"}]}]"#);
    let mut harness = Harness::new(Some(stream), redactor);
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

pub(super) async fn text_completion_failure(redactor: &Arc<Redactor>) {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "text_completion", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "text": "A"}]}
    ]"#,
    );
    let mut harness = Harness::new(Some(stream), redactor);
    harness.counter.fail_response_text = true;
    let (result, logs) = harness.run().await;
    assert!(matches!(failed(result), AssemblyError::TokenCount { .. }));
    assert_eq!(logs.len(), 2);
    assert_eq!(logs[0], "token_counter failed, assuming prompt tokens is 0");
    assert!(logs[1].ends_with(
        "Exception occurred - the token count failed: either text or messages must be provided"
    ));
}

pub(super) async fn text_total_overflow(redactor: &Arc<Redactor>) {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "text_completion", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "text": "A"}]}
    ]"#,
    );
    let mut harness = Harness::new(Some(stream), redactor);
    harness.messages = true;
    harness.counter.messages = i64::MAX;
    harness.counter.response_text = 1;
    let (result, logs) = harness.run().await;
    assert!(matches!(failed(result), AssemblyError::Overflow));
    assert!(logs[0].ends_with("Exception occurred - the token count overflowed"));
}

pub(super) async fn text_branch_selection(redactor: &Arc<Redactor>) {
    let lead = chunks(
        r#"[
        {"id": "chatcmpl-lead", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": []},
        {"choices": [{"index": 0, "text": "AB"}]}
    ]"#,
    );
    let mut harness = Harness::new(Some(lead), redactor);
    harness.messages = true;
    harness.counter.messages = 1;
    harness.counter.response_text = 1;
    let (result, _logs) = harness.run().await;
    let response = text(ok(result));
    assert_eq!(response.id, "chatcmpl-lead");
    assert_eq!(response.object, "text_completion");
    assert_eq!(response.choices[0].text, "AB");
}

pub(super) async fn text_branch_mixed(redactor: &Arc<Redactor>) {
    let mixed = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"content": "A"}}]},
        {"choices": [{"index": 0, "text": "B"}]}
    ]"#,
    );
    let mut harness = Harness::new(Some(mixed), redactor);
    harness.messages = true;
    harness.counter.response_text = 1;
    let response = chat_of(&harness).await;
    assert_eq!(response.object, "chat.completion");
    assert_eq!(response.choices[0].message.content.as_deref(), Some("A"));
}
