use betterlite_config::sections::hosted_chat_settings::Settings as HostedSettings;
use secrecy::{ExposeSecret as _, SecretString};
pub const FEATHERLESS_DEFAULT_API_BASE: &str = "https://api.featherless.ai/v1";
pub const AIML_DEFAULT_API_BASE: &str = "https://api.aimlapi.com/v1";
// LiteLLM chains the explicit value, each environment alias, then the default, skipping an empty one at every step
#[must_use]
pub fn featherless(
    settings: &HostedSettings,
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
) -> (String, Option<SecretString>) {
    (
        resolved_base(
            api_base,
            settings
                .featherless_ai_api_base
                .as_deref()
                .or(settings.featherless_api_base.as_deref()),
            FEATHERLESS_DEFAULT_API_BASE,
        ),
        resolved_key(
            api_key,
            settings
                .featherless_ai_api_key
                .as_ref()
                .or(settings.featherless_api_key.as_ref()),
        ),
    )
}
// AIML's chat identity reads only the primary key alias, never the image-side AIMLAPI_KEY
#[must_use]
pub fn aiml(
    settings: &HostedSettings,
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
) -> (String, Option<SecretString>) {
    (
        resolved_base(
            api_base,
            settings.aiml_api_base.as_deref(),
            AIML_DEFAULT_API_BASE,
        ),
        resolved_key(api_key, settings.aiml_api_key.as_ref()),
    )
}
fn resolved_base(explicit: Option<&str>, environment: Option<&str>, default: &str) -> String {
    explicit
        .filter(|value| !value.is_empty())
        .or(environment)
        .unwrap_or(default)
        .to_owned()
}
fn resolved_key(
    explicit: Option<&SecretString>,
    environment: Option<&SecretString>,
) -> Option<SecretString> {
    explicit
        .filter(|key| !key.expose_secret().is_empty())
        .cloned()
        .or_else(|| environment.cloned())
}
