use super::support::chunks;
use super::{Harness, chat_of, object, value};

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
    assert_eq!(
        response.choices[0]
            .message
            .tool_calls
            .as_ref()
            .map(Vec::len),
        Some(0)
    );
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
