use rstest::rstest;

use super::support::{FixedCounter, capture, messages, reply_body};
use crate::langflow_chat::error::{LangFlowError, NO_MESSAGE_MESSAGE};
use crate::langflow_chat::reply::{LangFlowChatResponse, ReplyInput, Usage, transform_response};

const MODEL: &str = "langflow/abc";

async fn reply_with(body: &str, status: u16) -> Result<LangFlowChatResponse, LangFlowError> {
    let messages = messages(r#"[{"role":"user","content":"hi"}]"#);
    transform_response(
        &ReplyInput {
            model: MODEL,
            status,
            body: body.as_bytes(),
            messages: &messages,
        },
        &FixedCounter::new(Some(1), Some(1)),
    )
    .await
}

async fn content(body: &str) -> String {
    reply_with(body, 200)
        .await
        .expect("the fixture body carries a message")
        .choice
        .message
        .content
}

#[tokio::test]
async fn the_reply_is_one_stop_choice_for_the_requested_model() {
    let response = reply_with(&reply_body("hello"), 200)
        .await
        .expect("the fixture body carries a message");
    assert_eq!(response.model, MODEL);
    assert_eq!(response.choice.index, 0);
    assert_eq!(response.choice.finish_reason, "stop");
    assert_eq!(response.choice.message.role, "assistant");
    assert_eq!(response.choice.message.content, "hello");
}

#[rstest]
#[case::results_message(
    r#"{"outputs":[{"outputs":[{"results":{"message":{"text":"hello"}}}]}]}"#,
    "hello"
)]
#[case::empty_results_falls_through(
    r#"{"outputs":[{"outputs":[{"results":{"message":{"text":""}},"outputs":{"x":{"message":{"text":"walk"}}}}]}]}"#,
    "walk"
)]
#[case::missing_results_walks_outputs(
    r#"{"outputs":[{"outputs":[{"outputs":{"x":{"message":{"text":"walk"}}}}]}]}"#,
    "walk"
)]
#[case::results_not_an_object_walks_outputs(
    r#"{"outputs":[{"outputs":[{"results":[],"outputs":{"x":{"message":{"text":"walk"}}}}]}]}"#,
    "walk"
)]
#[case::walk_skips_non_object_values(
    r#"{"outputs":[{"outputs":[{"outputs":{"a":7,"b":{"message":{"text":"second"}}}}]}]}"#,
    "second"
)]
#[case::walk_skips_values_without_a_message(
    r#"{"outputs":[{"outputs":[{"outputs":{"a":{},"b":{"message":{"text":"second"}}}}]}]}"#,
    "second"
)]
#[case::results_wins_over_the_walk(
    r#"{"outputs":[{"outputs":[{"results":{"message":{"text":"first"}},"outputs":{"x":{"message":{"text":"second"}}}}]}]}"#,
    "first"
)]
#[case::walk_keeps_the_object_order(
    r#"{"outputs":[{"outputs":[{"outputs":{"a":{"message":{"text":"one"}},"b":{"message":{"text":"two"}}}}]}]}"#,
    "one"
)]
#[tokio::test]
async fn content_comes_from_the_documented_paths(#[case] body: &str, #[case] expected: &str) {
    assert_eq!(content(body).await, expected);
}

#[rstest]
#[case::empty_object("{}")]
#[case::empty_outputs_list(r#"{"outputs":[]}"#)]
#[case::outputs_not_a_list(r#"{"outputs":{}}"#)]
#[case::first_output_not_an_object(r#"{"outputs":[1]}"#)]
#[case::empty_inner_outputs(r#"{"outputs":[{"outputs":[]}]}"#)]
#[case::empty_results(r#"{"outputs":[{"outputs":[{"results":{}}]}]}"#)]
#[case::empty_message_text(r#"{"outputs":[{"outputs":[{"results":{"message":{"text":""}}}]}]}"#)]
#[case::empty_outputs_map(r#"{"outputs":[{"outputs":[{"outputs":{}}]}]}"#)]
#[case::second_output_ignored(
    r#"{"outputs":[{"outputs":[{"results":{"message":{"text":""}}}]},{"outputs":[{"results":{"message":{"text":"second"}}}]}]}"#
)]
#[case::json_array("[]")]
#[case::json_null("null")]
#[case::json_string(r#""hello""#)]
#[tokio::test]
async fn a_body_without_a_message_raises_the_extraction_error(#[case] body: &str) {
    let error = reply_with(body, 200)
        .await
        .expect_err("the fixture body carries no message");
    assert!(matches!(
        error,
        LangFlowError::Api { status_code: 500, message } if message == NO_MESSAGE_MESSAGE
    ));
}

#[rstest]
#[case::ok_status(200)]
#[case::error_status(502)]
#[tokio::test]
async fn a_non_json_body_keeps_the_http_status(#[case] status: u16) {
    let error = reply_with("not json", status)
        .await
        .expect_err("a non-JSON body has no message");
    match error {
        LangFlowError::Api {
            status_code,
            message,
        } => {
            assert_eq!(status_code, status);
            assert!(
                message.starts_with("LangFlow returned a non-JSON response: "),
                "unexpected message: {message}"
            );
        }
        other @ (LangFlowError::ApiBaseRequired
        | LangFlowError::Environment { .. }
        | LangFlowError::FlowId { .. }) => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn usage_counts_with_the_requested_model() {
    let counter = FixedCounter::new(Some(3), Some(5));
    let messages = messages(r#"[{"role":"user","content":"hi"}]"#);
    let response = transform_response(
        &ReplyInput {
            model: MODEL,
            status: 200,
            body: reply_body("hello there").as_bytes(),
            messages: &messages,
        },
        &counter,
    )
    .await
    .expect("the fixture body carries a message");
    assert_eq!(
        response.usage,
        Some(Usage {
            prompt_tokens: 3,
            completion_tokens: 5,
            total_tokens: 8,
        })
    );
    assert_eq!(counter.models(), [String::from(MODEL), String::from(MODEL)]);
}

#[tokio::test]
async fn a_failed_prompt_count_leaves_usage_unset() {
    let (logs, _guard) = capture();
    let messages = messages(r#"[{"role":"user","content":"hi"}]"#);
    let response = transform_response(
        &ReplyInput {
            model: MODEL,
            status: 200,
            body: reply_body("hello").as_bytes(),
            messages: &messages,
        },
        &FixedCounter::new(None, Some(5)),
    )
    .await
    .expect("the fixture body carries a message");
    assert_eq!(response.usage, None);
    assert!(
        logs.text()
            .contains("Failed to calculate token usage: either text or messages must be provided")
    );
}

#[tokio::test]
async fn a_failed_completion_count_leaves_usage_unset() {
    let (logs, _guard) = capture();
    let messages = messages(r#"[{"role":"user","content":"hi"}]"#);
    let response = transform_response(
        &ReplyInput {
            model: MODEL,
            status: 200,
            body: reply_body("hello").as_bytes(),
            messages: &messages,
        },
        &FixedCounter::new(Some(3), None),
    )
    .await
    .expect("the fixture body carries a message");
    assert_eq!(response.usage, None);
    assert!(
        logs.text()
            .contains("Failed to calculate token usage: either text or messages must be provided")
    );
}

#[tokio::test]
async fn an_overflowing_total_leaves_usage_unset() {
    let (logs, _guard) = capture();
    let messages = messages(r#"[{"role":"user","content":"hi"}]"#);
    let response = transform_response(
        &ReplyInput {
            model: MODEL,
            status: 200,
            body: reply_body("hello").as_bytes(),
            messages: &messages,
        },
        &FixedCounter::new(Some(u64::MAX), Some(1)),
    )
    .await
    .expect("the fixture body carries a message");
    assert_eq!(response.usage, None);
    assert!(logs.text().contains(
        "Failed to calculate token usage: the token count does not fit in a machine word"
    ));
}
