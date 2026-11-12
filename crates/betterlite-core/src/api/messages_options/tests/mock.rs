use betterlite_error::error::ErrorClass;
use rstest::rstest;

use super::super::INCOMPLETE_STREAM_MESSAGE;
use super::super::error::MessagesOptionsError;
use super::super::mock::{MockMessage, MockTextBlock, MockUsage, mock_message};

fn fixed(text: &str) -> MockMessage {
    MockMessage {
        content: vec![MockTextBlock {
            text: text.to_owned(),
            r#type: "text",
        }],
        id: "msg_013Zva2CMHLNnXjNJJKqJ2EF",
        model: "claude-sonnet-4-20250514",
        role: "assistant",
        stop_reason: "end_turn",
        stop_sequence: None,
        r#type: "message",
        usage: MockUsage {
            input_tokens: 2095,
            output_tokens: 503,
        },
    }
}

#[rstest]
#[case::default_text(None, "Hi! My name is Claude.")]
#[case::custom_text(Some("pong"), "pong")]
#[case::empty_text(Some(""), "")]
fn the_mock_message_is_fixed_apart_from_its_text(
    #[case] mock_response: Option<&str>,
    #[case] text: &str,
) {
    assert_eq!(
        mock_message("claude-opus-4-8", mock_response).expect("mock message"),
        fixed(text)
    );
}

#[test]
fn the_mock_message_serializes_to_the_anthropic_shape() {
    let wire = serde_json::to_string(&mock_message("any", None).expect("mock")).expect("json");
    assert_eq!(
        wire,
        r#"{"content":[{"text":"Hi! My name is Claude.","type":"text"}],"id":"msg_013Zva2CMHLNnXjNJJKqJ2EF","model":"claude-sonnet-4-20250514","role":"assistant","stop_reason":"end_turn","stop_sequence":null,"type":"message","usage":{"input_tokens":2095,"output_tokens":503}}"#
    );
}

#[rstest]
#[case::internal_server(
    "litellm.InternalServerError",
    "this is a mock internal server error",
    500
)]
#[case::context_window(
    "litellm.ContextWindowExceededError",
    "this is a mock context window exceeded error",
    400
)]
#[case::rate_limit("litellm.RateLimitError", "this is a mock rate limit error", 429)]
fn mock_error_literals_raise_the_matching_sdk_error(
    #[case] literal: &str,
    #[case] message: &str,
    #[case] status: u16,
) {
    let error = mock_message("claude-opus-4-8", Some(literal)).unwrap_err();
    assert_eq!(error.to_string(), message);
    let class = error.classify().expect("sdk class");
    assert_eq!(class.metadata().status_code, status);
    match (&error, &class) {
        (
            MessagesOptionsError::MockInternalServer { model },
            ErrorClass::InternalServer(context),
        ) => {
            assert_eq!(model, "claude-opus-4-8");
            assert_eq!(context.model.as_deref(), Some("claude-opus-4-8"));
            assert_eq!(context.provider.as_deref(), Some("anthropic"));
        }
        (
            MessagesOptionsError::MockContextWindowExceeded { model },
            ErrorClass::ContextWindowExceeded(_),
        )
        | (MessagesOptionsError::MockRateLimit { model }, ErrorClass::RateLimit(_)) => {
            assert_eq!(model, "claude-opus-4-8");
        }
        _ => panic!("literal {literal} produced the wrong error"),
    }
}

#[rstest]
#[case::lowercase("litellm.internalservererror")]
#[case::padded(" litellm.RateLimitError")]
#[case::prefix("litellm.RateLimitError!")]
fn near_miss_literals_are_plain_text(#[case] literal: &str) {
    assert_eq!(
        mock_message("m", Some(literal)).expect("plain text"),
        fixed(literal)
    );
}

#[test]
fn the_incomplete_stream_message_is_verbatim() {
    assert_eq!(
        INCOMPLETE_STREAM_MESSAGE,
        "Provider stream ended before emitting a message_stop event; the response is incomplete and any partial content (e.g. tool_use input JSON) may be truncated."
    );
}
