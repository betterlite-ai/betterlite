use secrecy::{ExposeSecret as _, SecretString};

use super::DEFAULT_API_BASE;

// LiteLLM chains the explicit value, the GROQ_* environment value, then the default, with an empty string counting as unset at each step
#[must_use]
pub fn compatible_provider_info(
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
    environment_base: Option<&str>,
    environment_key: Option<&SecretString>,
) -> (String, Option<SecretString>) {
    let base = api_base
        .filter(|value| !value.is_empty())
        .or(environment_base.filter(|value| !value.is_empty()))
        .unwrap_or(DEFAULT_API_BASE)
        .to_owned();
    let key = api_key
        .filter(|key| !key.expose_secret().is_empty())
        .or_else(|| environment_key.filter(|key| !key.expose_secret().is_empty()))
        .cloned();
    (base, key)
}
