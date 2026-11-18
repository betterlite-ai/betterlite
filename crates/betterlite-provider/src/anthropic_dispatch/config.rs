use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use std::str::FromStr as _;

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

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigSelectionError {
    #[error("'{provider}' is not a valid LlmProviders")]
    InvalidProvider { provider: String },
    #[error("Provider config not found for model: {model} and provider: {provider}")]
    NotFound { model: String, provider: String },
}

pub fn selected_config(
    model: &str,
    provider: &str,
    selection: Option<ChatConfigTag>,
) -> Result<ChatConfigTag, ConfigSelectionError> {
    let id = ProviderId::from_str(provider).map_err(|_invalid| {
        ConfigSelectionError::InvalidProvider {
            provider: provider.to_owned(),
        }
    })?;
    if matches!(
        id,
        ProviderId::Abliteration
            | ProviderId::Llamagate
            | ProviderId::Gmi
            | ProviderId::Sarvam
            | ProviderId::Aihubmix
            | ProviderId::Crusoe
            | ProviderId::Empiriolabs
            | ProviderId::Veniceai
    ) {
        return Err(ConfigSelectionError::InvalidProvider {
            provider: provider.to_owned(),
        });
    }
    selection.ok_or_else(|| ConfigSelectionError::NotFound {
        model: model.to_owned(),
        provider: provider.to_owned(),
    })
}

#[path = "config/tests.rs"]
mod tests;
