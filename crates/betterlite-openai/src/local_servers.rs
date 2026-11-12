pub mod docker_model_runner;
pub mod error;
pub mod lemonade_cost;
pub mod llamafile;
pub mod lm_studio;
pub mod oobabooga;
pub mod xinference_image;

mod tests;

use secrecy::{ExposeSecret as _, SecretString};

#[derive(Debug, Default)]
pub struct ProviderEnvironment {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
}

#[derive(Debug)]
pub struct ProviderInfo {
    pub api_base: String,
    pub api_key: SecretString,
}

// an empty string falls through every "a or b or c" chain, as the compatibility policy says
#[must_use]
pub fn resolve_api_base(
    explicit: Option<&str>,
    environment: &ProviderEnvironment,
) -> Option<String> {
    explicit
        .filter(|base| !base.is_empty())
        .or_else(|| {
            environment
                .api_base
                .as_deref()
                .filter(|base| !base.is_empty())
        })
        .map(str::to_owned)
}

#[must_use]
pub fn resolve_api_key(
    explicit: Option<&SecretString>,
    environment: &ProviderEnvironment,
    placeholder: &str,
) -> SecretString {
    let key = explicit
        .filter(|key| !key.expose_secret().is_empty())
        .or_else(|| {
            environment
                .api_key
                .as_ref()
                .filter(|key| !key.expose_secret().is_empty())
        })
        .map_or(placeholder, |key| key.expose_secret());
    SecretString::from(key.to_owned())
}
