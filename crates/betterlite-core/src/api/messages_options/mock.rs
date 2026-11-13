use serde::Serialize;

use super::error::MessagesOptionsError;

const DEFAULT_MOCK_TEXT: &str = "Hi! My name is Claude.";

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct MockTextBlock {
    pub text: String,
    pub r#type: &'static str,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct MockUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct MockMessage {
    pub content: Vec<MockTextBlock>,
    pub id: &'static str,
    pub model: &'static str,
    pub role: &'static str,
    pub stop_reason: &'static str,
    pub stop_sequence: Option<String>,
    pub r#type: &'static str,
    pub usage: MockUsage,
}

// The model, ids and token counts are fixed whatever model and max_tokens the caller sent.
pub fn mock_message(
    model: &str,
    mock_response: Option<&str>,
) -> Result<MockMessage, MessagesOptionsError> {
    let text = mock_response.unwrap_or(DEFAULT_MOCK_TEXT);
    match text {
        "litellm.InternalServerError" => Err(MessagesOptionsError::MockInternalServer {
            model: model.to_owned(),
        }),
        "litellm.ContextWindowExceededError" => {
            Err(MessagesOptionsError::MockContextWindowExceeded {
                model: model.to_owned(),
            })
        }
        "litellm.RateLimitError" => Err(MessagesOptionsError::MockRateLimit {
            model: model.to_owned(),
        }),
        _ => Ok(MockMessage {
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
        }),
    }
}
