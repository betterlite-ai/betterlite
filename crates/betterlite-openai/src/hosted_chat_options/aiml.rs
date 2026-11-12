use super::resolved_identity;
use betterlite_config::sections::hosted_chat_settings::Settings as HostedSettings;
use secrecy::SecretString;

// AIML's chat identity reads only the primary key alias, never the image-side AIMLAPI_KEY
#[must_use]
pub fn chat_identity(
    settings: &HostedSettings,
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
) -> (String, Option<SecretString>) {
    resolved_identity(
        api_base,
        api_key,
        settings.aiml_api_base.as_deref(),
        settings.aiml_api_key.as_ref(),
        "https://api.aimlapi.com/v1",
    )
}
