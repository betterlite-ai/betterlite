use super::hub::LLAMA3_INSTRUCT_MODEL;
use super::{Output, PromptFactory};
use crate::builtin::{self, Message};
use crate::error::FactoryError;
use chrono::{DateTime, Utc};

pub(super) async fn dispatch(
    factory: &PromptFactory,
    lower: &str,
    messages: &[Message],
    now: DateTime<Utc>,
) -> Option<Result<Output, FactoryError>> {
    if lower.contains("amazon.titan-text") {
        Some(
            builtin::families::anthropic(messages, builtin::families::Style::Titan)
                .map(Output::Text)
                .map_err(FactoryError::from),
        )
    } else if lower.contains("anthropic.") {
        Some(claude_variant(lower, messages))
    } else if lower.contains("mistral.") {
        Some(
            builtin::mistral(messages)
                .map(Output::Text)
                .map_err(FactoryError::from),
        )
    } else if lower.contains("llama2") && lower.contains("chat") {
        Some(
            builtin::llama2(messages)
                .map(Output::Text)
                .map_err(FactoryError::from),
        )
    } else if (lower.contains("llama3") || lower.contains("llama4")) && lower.contains("instruct") {
        Some(
            factory
                .hf_chat_template(LLAMA3_INSTRUCT_MODEL, messages, now)
                .await
                .map(Output::Text),
        )
    } else {
        None
    }
}

fn claude_variant(lower: &str, messages: &[Message]) -> Result<Output, FactoryError> {
    if lower.contains("claude-2.1") || lower.contains("claude-v2:1") {
        builtin::families::anthropic(messages, builtin::families::Style::Claude21)
            .map(Output::Text)
            .map_err(FactoryError::from)
    } else {
        builtin::families::anthropic(messages, builtin::families::Style::Claude)
            .map(Output::Text)
            .map_err(FactoryError::from)
    }
}

mod tests;
