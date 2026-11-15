use crate::streamed_response_assembly::AssemblyError;

use super::support::chunks;
use super::{FIXED_NOW, Harness, chat_of, failed, ok, text, value};

pub(super) async fn text_completion_assembly() {
    text_completion_join().await;
    text_defaults().await;
    text_completion_failure().await;
    text_total_overflow().await;
    text_branch_selection().await;
    text_branch_mixed().await;
}

pub(super) async fn text_completion_join() {
    let logprobs = value(
        r#"{"tokens": ["Hey"], "token_logprobs": [-0.1], "top_logprobs": [{}], "text_offset": [0]}"#,
    );
    let stream = chunks(
        r#"[{"id": "chatcmpl-text", "object": "text_completion", "created": 1700000000, "model": "gpt-4o",
         "system_fingerprint": "fp_1", "choices": [{"index": 0, "text": "Hey"}]},
        {"created": 1700000500, "choices": [{"index": 0, "text": " you", "finish_reason": "length"}, {"index": 1, "text": "!"}]},
        {"created": 1700000999, "choices": [{"index": 0, "text": null, "finish_reason": "stop",
         "logprobs": {"tokens": ["Hey"], "token_logprobs": [-0.1], "top_logprobs": [{}], "text_offset": [0]}}]}]"#,
    );
    let mut harness = Harness::new(Some(stream));
    harness.messages = true;
    harness.prompt = Some(Box::new(|| Ok(7)));
    harness.counter.messages = 3;
    harness.counter.response_text = 2;
    harness.now = 1_800_000_000;
    let (result, logs) = harness.run().await;
    let response = text(ok(result));
    assert_eq!(response.id, "chatcmpl-text");
    assert_eq!(response.object, "text_completion");
    assert_eq!(response.created, 1_700_000_000);
    assert_eq!(response.model.as_deref(), Some("gpt-4o"));
    assert_eq!(response.system_fingerprint.as_deref(), Some("fp_1"));
    assert_eq!(response.choices.len(), 1);
    assert_eq!(response.choices[0].text, "Hey you!");
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
}

pub(super) async fn text_branch_mixed() {
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
