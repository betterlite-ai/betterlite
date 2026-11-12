use betterlite_config::sections::hosted_chat_settings::Settings as HostedSettings;
use secrecy::{ExposeSecret as _, SecretString};

pub const DEFAULT_API_BASE: &str = "https://api.aimlapi.com/v1";

// AIML's chat identity reads only the primary key alias, never the image-side AIMLAPI_KEY
#[must_use]
pub fn resolve_provider_info(
    settings: &HostedSettings,
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
) -> (String, Option<SecretString>) {
    let base = api_base
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            settings
                .aiml_api_base
                .as_ref()
                .filter(|value| !value.is_empty())
                .cloned()
        })
        .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
    let environment_key = settings
        .aiml_api_key
        .as_ref()
        .filter(|key| !key.expose_secret().is_empty());
    let key = api_key
        .filter(|key| !key.expose_secret().is_empty())
        .cloned()
        .or_else(|| environment_key.cloned());
    (base, key)
}
