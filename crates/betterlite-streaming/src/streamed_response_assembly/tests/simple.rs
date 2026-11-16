use super::support::chunks;
use super::{Harness, chat_of, response_of};

pub(super) async fn simple_text_path() {
    simple_text_join().await;
    simple_delta_shapes().await;
    simple_first_choice_content().await;
    simple_disabled_by_fields().await;
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

pub(super) async fn simple_delta_shapes() {
    let unsupported = r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"content": "A"}}]},
        {"choices": [{"index": 0, "delta": null}]},
        {"choices": [{"index": 0, "delta": "text"}]},
        {"choices": [{"index": 0, "delta": [1, 2]}]}
    ]"#;
    let later_choice_tools = r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"role": "assistant", "content": "A"}}]},
        {"choices": [
            {"index": 0, "delta": {"content": "B"}},
            {"index": 1, "delta": {"tool_calls": []}}
        ]}
    ]"#;
    // Unsupported deltas keep the fast path; an empty tool_calls on a later choice leaves it and still joins an empty list.
    let cases = [
        (unsupported, "A", 1, None),
        (later_choice_tools, "AB", 2, Some(0)),
    ];
    for (source, content, text_tokens, tool_calls) in cases {
        let response = response_of(source, text_tokens).await;
        let message = &response.choices[0].message;
        assert_eq!(message.content.as_deref(), Some(content));
        assert_eq!(message.tool_calls.as_ref().map(Vec::len), tool_calls);
        assert_eq!(
            response.usage.completion_tokens_details.is_some(),
            tool_calls.is_none(),
            "{content}"
        );
    }
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

const DISABLING: [&str; 8] = [
    r#""tool_calls": []"#,
    r#""function_call": {"name": "f", "arguments": ""}"#,
    r#""reasoning_content": """#,
    r#""thinking_blocks": []"#,
    r#""annotations": []"#,
    r#""audio": {}"#,
    r#""images": []"#,
    r#""provider_specific_fields": {}"#,
];

const DISABLED_STREAM: &str = r#"[
    {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
     "choices": [{"index": 0, "delta": {"role": "assistant", "content": "A"}}]},
    {"choices": [{"index": 0, "delta": {"content": "B", FIELD}}]}
]"#;

// Any of these fields, even empty, sends the stream through the complex path, which leaves no zero reasoning stamp.
pub(super) async fn simple_disabled_by_fields() {
    for field in DISABLING {
        let response = response_of(&DISABLED_STREAM.replace("FIELD", field), 2).await;
        let content = response.choices[0].message.content.as_deref();
        assert_eq!(content, Some("AB"), "{field}");
        assert!(
            response.usage.completion_tokens_details.is_none(),
            "{field}"
        );
    }
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
