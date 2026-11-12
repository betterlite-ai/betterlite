use betterlite_config::sections::studio_identity::Settings;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use super::API_REVISION;

pub type RequestHeaders = IndexMap<String, SecretString>;

#[derive(Debug, Clone, Default)]
pub struct StudioParams {
    pub api_key: Option<SecretString>,
}

// An empty key or base counts as unset, as `api_key or env` does in LiteLLM.
#[must_use]
pub fn resolve_api_key(params: &StudioParams, settings: &Settings) -> Option<SecretString> {
    params
        .api_key
        .as_ref()
        .filter(|key| !key.expose_secret().is_empty())
        .or(settings.environment_key.as_ref())
        .cloned()
}

#[must_use]
pub fn resolve_api_base(api_base: Option<&str>, settings: &Settings) -> String {
    api_base
        .filter(|base| !base.is_empty())
        .map_or_else(|| settings.api_base.clone(), str::to_owned)
}

#[must_use]
pub fn validate_environment(
    caller: &IndexMap<String, String>,
    params: Option<&StudioParams>,
    settings: &Settings,
) -> RequestHeaders {
    let mut headers: RequestHeaders = caller
        .iter()
        .map(|(name, value)| (name.clone(), SecretString::from(value.as_str())))
        .collect();
    headers.insert(
        "Content-Type".to_owned(),
        SecretString::from("application/json"),
    );
    if let Some(key) = params.and_then(|given| resolve_api_key(given, settings)) {
        headers.insert("x-goog-api-key".to_owned(), key);
    }
    headers.insert("Api-Revision".to_owned(), SecretString::from(API_REVISION));
    headers
}
