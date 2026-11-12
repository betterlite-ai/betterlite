pub mod error;
pub mod metadata;
pub mod plan;
pub mod urls;

mod tests;

use betterlite_config::sections::studio_identity::{DEFAULT_API_BASE, Settings};
use secrecy::{ExposeSecret as _, SecretString};

pub const SUPPORTED_OPENAI_PARAMS: [&str; 0] = [];

// The litellm_params entries the file transformations read; everything else travels untouched.
#[derive(Debug, Clone, Default)]
pub struct RequestParams {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
}

// LiteLLM's `a or b or env`: an empty key counts as unset, and the environment key is the last candidate.
pub(crate) fn resolve_key(
    candidates: &[Option<&SecretString>],
    settings: &Settings,
) -> Option<SecretString> {
    for candidate in candidates {
        let found = candidate
            .filter(|key| !key.expose_secret().is_empty())
            .cloned();
        if let Some(key) = found {
            return Some(key);
        }
    }
    settings.environment_key.clone()
}

// get_api_base ends at a hardcoded default, so even an empty configured base still yields a URL.
pub(crate) fn resolve_base(api_base: Option<&str>, settings: &Settings) -> String {
    let candidate = api_base
        .filter(|base| !base.is_empty())
        .unwrap_or(&settings.api_base);
    if candidate.is_empty() {
        DEFAULT_API_BASE.to_owned()
    } else {
        candidate.to_owned()
    }
}
