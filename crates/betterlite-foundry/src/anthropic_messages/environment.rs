use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::sync::Arc;

use betterlite_azure::azure_urls::{EnvironmentHeadersRequest, validate_azure_environment};
use betterlite_azure::credentials::source::{CallableTokenProvider, CredentialSources};
use betterlite_config::sections::azure_credentials::DeploymentSettings;
use secrecy::{ExposeSecret as _, SecretString};

use super::environment_error::EnvironmentError;

pub type Headers = BTreeMap<String, String>;

const AZURE_KEY_HEADER: &str = "api-key";
const ANTHROPIC_KEY_HEADER: &str = "x-api-key";
const ANTHROPIC_VERSION_HEADER: &str = "anthropic-version";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const CONTENT_TYPE_HEADER: &str = "content-type";
const JSON_CONTENT_TYPE: &str = "application/json";

// The api_key field of the litellm_params the caller passed, in the shape LiteLLM received it
#[derive(Debug, Clone, Copy)]
pub enum ParamsKey<'data> {
    DictAbsent,
    DictNull,
    Dict(&'data SecretString),
    Typed(Option<&'data SecretString>),
}

pub struct TokenSource<'data> {
    pub credentials: &'data DeploymentSettings,
    pub refresh_enabled: bool,
    pub provider: Option<Arc<dyn CallableTokenProvider + 'data>>,
    pub sources: &'data dyn CredentialSources,
}

pub struct EnvironmentRequest<'data> {
    pub headers: Headers,
    pub params_key: ParamsKey<'data>,
    pub argument_key: Option<&'data SecretString>,
    // litellm.api_key, litellm.azure_key and the Azure key variables, resolved by the caller
    pub configured_key: Option<&'data SecretString>,
    pub token: TokenSource<'data>,
    pub api_base: Option<String>,
}

// A dict only takes the argument when it has no api_key entry; a typed object when its field is empty
fn explicit_key<'data>(
    params_key: ParamsKey<'data>,
    argument: Option<&'data SecretString>,
) -> Option<&'data SecretString> {
    let usable = argument.filter(|key| !key.expose_secret().is_empty());
    match params_key {
        ParamsKey::DictAbsent => usable,
        ParamsKey::DictNull => None,
        ParamsKey::Dict(key) => Some(key),
        ParamsKey::Typed(Some(key)) if !key.expose_secret().is_empty() => Some(key),
        ParamsKey::Typed(_) => usable,
    }
}

// The beta step is a closure because betterlite-foundry cannot depend on betterlite-core
pub fn validate_anthropic_messages_environment<B, U>(
    request: EnvironmentRequest<'_>,
    update_betas: U,
) -> Result<(Headers, Option<String>), EnvironmentError<B>>
where
    B: StdError + 'static,
    U: FnOnce(Headers) -> Result<Headers, B>,
{
    let explicit = explicit_key(request.params_key, request.argument_key)
        .filter(|key| !key.expose_secret().is_empty());
    let mut headers = validate_azure_environment(EnvironmentHeadersRequest {
        headers: request.headers,
        api_key: explicit.or(request.configured_key),
        credentials: request.token.credentials,
        refresh_enabled: request.token.refresh_enabled,
        provider: request.token.provider,
        sources: request.token.sources,
    })?;
    if !headers.contains_key(ANTHROPIC_KEY_HEADER)
        && let Some(key) = headers.remove(AZURE_KEY_HEADER)
    {
        headers.insert(ANTHROPIC_KEY_HEADER.to_owned(), key);
    }
    headers
        .entry(ANTHROPIC_VERSION_HEADER.to_owned())
        .or_insert_with(|| ANTHROPIC_VERSION.to_owned());
    headers
        .entry(CONTENT_TYPE_HEADER.to_owned())
        .or_insert_with(|| JSON_CONTENT_TYPE.to_owned());
    let with_betas = update_betas(headers).map_err(EnvironmentError::Betas)?;
    Ok((with_betas, request.api_base))
}
