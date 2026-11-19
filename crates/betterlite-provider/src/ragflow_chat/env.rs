use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_secret_managers::secret_lookup::SecretLookup;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::RagflowError;
use super::parse::parse_model;

pub const API_BASE_ENV: &str = "RAGFLOW_API_BASE";
pub const API_KEY_ENV: &str = "RAGFLOW_API_KEY";
pub const PROVIDER: &str = "ragflow";
pub const STORED_MODEL_FIELD: &str = "_ragflow_actual_model";

const AUTHORIZATION: &str = "Authorization";
const CONTENT_TYPE: &str = "Content-Type";
const LOWER_CONTENT_TYPE: &str = "content-type";
const JSON_CONTENT_TYPE: &str = "application/json";

// LiteLLM's module-level SDK settings, which the base and key chains fall back to
#[derive(Debug, Clone, Copy, Default)]
pub struct Globals<'request> {
    pub api_base: Option<&'request str>,
    pub api_key: Option<&'request SecretString>,
}

pub struct CredentialsInput<'request> {
    pub model: &'request str,
    pub api_base: Option<&'request str>,
    pub api_key: Option<&'request SecretString>,
    pub globals: Globals<'request>,
    pub secrets: &'request SecretLookup,
    pub environment: &'request BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct ProviderInfo {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
}

mod tests;

// LiteLLM's _get_openai_compatible_provider_info: the model parses before any base or key is read
pub fn provider_info(input: &CredentialsInput<'_>) -> Result<ProviderInfo, RagflowError> {
    parse_model(input.model)?;
    Ok(ProviderInfo {
        api_base: resolve_api_base(input)?,
        api_key: resolve_api_key(input)?,
    })
}

// the first explicit source that is present and not empty
fn first_present(candidates: [Option<&str>; 2]) -> Option<&str> {
    candidates
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty())
}

// LiteLLM's base chain, where an absent or empty value falls through to the next source
pub(super) fn resolve_api_base(
    input: &CredentialsInput<'_>,
) -> Result<Option<String>, RagflowError> {
    match first_present([input.api_base, input.globals.api_base]) {
        Some(base) => Ok(Some(base.to_owned())),
        None => Ok(secret_text(input, API_BASE_ENV)?.map(|text| text.expose_secret().to_owned())),
    }
}

// LiteLLM's key chain, where an absent or empty key falls through, and an empty variable counts as unset
fn resolve_api_key(input: &CredentialsInput<'_>) -> Result<Option<SecretString>, RagflowError> {
    let explicit = input
        .api_key
        .filter(|key| !key.expose_secret().is_empty())
        .or_else(|| {
            input
                .globals
                .api_key
                .filter(|key| !key.expose_secret().is_empty())
        });
    explicit.map_or_else(
        || secret_text(input, API_KEY_ENV),
        |key| Ok(Some(key.clone())),
    )
}

fn secret_text(
    input: &CredentialsInput<'_>,
    name: &'static str,
) -> Result<Option<SecretString>, RagflowError> {
    let value = input
        .secrets
        .get_secret_str(input.environment, name, None)
        .map_err(|source| RagflowError::Environment { name, source })?;
    Ok(value.filter(|text| !text.expose_secret().is_empty()))
}

// LiteLLM's validate_environment: the key header, the default content type, then the parsed model stored for the body
pub fn validate_environment(
    input: &CredentialsInput<'_>,
    headers: &mut BTreeMap<String, String>,
    litellm_params: &mut Object,
) -> Result<(), RagflowError> {
    if let Some(key) = resolve_api_key(input)? {
        headers.insert(
            AUTHORIZATION.to_owned(),
            format!("Bearer {}", key.expose_secret()),
        );
    }
    if !headers.contains_key(LOWER_CONTENT_TYPE) && !headers.contains_key(CONTENT_TYPE) {
        headers.insert(CONTENT_TYPE.to_owned(), JSON_CONTENT_TYPE.to_owned());
    }
    // a bad model name is swallowed here, and the URL step reports it later
    if let Ok(parsed) = parse_model(input.model) {
        litellm_params.insert(STORED_MODEL_FIELD.to_owned(), Json::Text(parsed.model));
    }
    Ok(())
}
