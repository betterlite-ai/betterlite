use std::str::FromStr;

use betterlite_models::identity::provider_id::ProviderId;
use thiserror::Error;

// litellm/llms/anthropic/__init__.py: the batches config only for the results route of the batches API.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnthropicConfigKind {
    Anthropic,
    AnthropicBatches,
}

#[must_use]
pub fn get_anthropic_config(url_route: &str) -> AnthropicConfigKind {
    if url_route.contains("messages/batches") && url_route.contains("results") {
        AnthropicConfigKind::AnthropicBatches
    } else {
        AnthropicConfigKind::Anthropic
    }
}

// The chat config the Anthropic handler can run; the shared selector answers with one of these when it finds one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChatConfigKind {
    Anthropic,
    AnthropicText,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigSelectionError {
    #[error("'{provider}' is not a valid LlmProviders value")]
    InvalidProvider { provider: String },
    #[error("Provider config not found for model: {model} and provider: {provider}")]
    NotFound { model: String, provider: String },
}

// LlmProviders(custom_llm_provider) first, then the shared selector; a missing answer is main.py's ValueError.
pub fn select_chat_config<F>(
    model: &str,
    provider: &str,
    select: F,
) -> Result<ChatConfigKind, ConfigSelectionError>
where
    F: FnOnce(ProviderId) -> Option<ChatConfigKind>,
{
    let parsed = ProviderId::from_str(provider).map_err(|_invalid| {
        ConfigSelectionError::InvalidProvider {
            provider: provider.to_owned(),
        }
    })?;
    select(parsed).ok_or_else(|| ConfigSelectionError::NotFound {
        model: model.to_owned(),
        provider: provider.to_owned(),
    })
}

// The closed factory: only the two Anthropic chat configs belong to this adapter's class.
#[must_use]
pub const fn anthropic_chat_config(provider: ProviderId) -> Option<ChatConfigKind> {
    match provider {
        ProviderId::Anthropic => Some(ChatConfigKind::Anthropic),
        ProviderId::AnthropicText => Some(ChatConfigKind::AnthropicText),
        _ => None,
    }
}

#[path = "config/tests.rs"]
mod tests;
