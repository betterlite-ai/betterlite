use super::Output;
use crate::builtin::{self, Message};
use crate::error::FactoryError;

pub(super) fn dispatch(lower: &str, messages: &[Message]) -> Option<Result<Output, FactoryError>> {
    if lower.contains("amazon.titan-text") {
        Some(
            builtin::titan(messages)
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
        Some(Err(FactoryError::NoChatTemplate))
    } else {
        None
    }
}

fn claude_variant(lower: &str, messages: &[Message]) -> Result<Output, FactoryError> {
    if lower.contains("claude-2.1") || lower.contains("claude-v2:1") {
        builtin::claude_2_1(messages)
            .map(Output::Text)
            .map_err(FactoryError::from)
    } else {
        builtin::claude(messages)
            .map(Output::Text)
            .map_err(FactoryError::from)
    }
}

mod tests;
