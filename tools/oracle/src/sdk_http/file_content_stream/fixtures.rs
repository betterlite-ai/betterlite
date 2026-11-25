use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_config::sections::anthropic_credentials::Settings as AnthropicCredentials;

pub(crate) mod azure;
pub(crate) mod content;
pub(crate) mod logging;
pub(crate) mod setup;
pub(crate) mod transport;

pub(crate) fn anthropic_settings(credentials: &AnthropicCredentials) -> AnthropicSettings {
    AnthropicSettings {
        api_key: credentials.api_key.clone(),
        auth_token: credentials.auth_token.clone(),
        api_base: credentials.api_base.clone(),
        use_bearer_for_custom_base: credentials.use_bearer_for_custom_base,
        anthropic_version: AnthropicSettings::default().anthropic_version,
    }
}
