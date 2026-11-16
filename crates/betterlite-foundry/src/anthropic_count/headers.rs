use betterlite_anthropic::credentials::ANTHROPIC_VERSION;
use betterlite_azure::azure_urls::{EnvironmentHeadersRequest, validate_azure_environment};
use betterlite_azure::credentials::selection::AzureTokenError;
use secrecy::{ExposeSecret as _, SecretString};

use crate::anthropic_messages::environment::{Headers, ParamsKey, TokenSource, explicit_key};

const CONTENT_TYPE_HEADER: &str = "Content-Type";
const CONTENT_TYPE_JSON: &str = "application/json";
const ANTHROPIC_VERSION_HEADER: &str = "anthropic-version";
const ANTHROPIC_BETA_HEADER: &str = "anthropic-beta";
const ANTHROPIC_KEY_HEADER: &str = "x-api-key";

pub struct HeadersRequest<'call> {
    pub api_key: &'call SecretString,
    pub params_key: ParamsKey<'call>,
    pub configured_key: Option<&'call SecretString>,
    pub token: &'call TokenSource<'call>,
}

pub fn get_required_headers(
    request: HeadersRequest<'_>,
    beta_version: &str,
) -> Result<Headers, AzureTokenError> {
    let mut headers = Headers::new();
    headers.insert(CONTENT_TYPE_HEADER.to_owned(), CONTENT_TYPE_JSON.to_owned());
    headers.insert(
        ANTHROPIC_VERSION_HEADER.to_owned(),
        ANTHROPIC_VERSION.to_owned(),
    );
    headers.insert(ANTHROPIC_BETA_HEADER.to_owned(), beta_version.to_owned());
    headers.insert(
        ANTHROPIC_KEY_HEADER.to_owned(),
        request.api_key.expose_secret().to_owned(),
    );
    let azure_key = explicit_key(request.params_key, Some(request.api_key))
        .filter(|key| !key.expose_secret().is_empty())
        .or(request.configured_key);
    let azure = validate_azure_environment(EnvironmentHeadersRequest {
        headers: Headers::new(),
        api_key: azure_key,
        credentials: request.token.credentials,
        refresh_enabled: request.token.refresh_enabled,
        provider: request.token.provider.clone(),
        sources: request.token.sources,
    })?;
    headers.extend(azure);
    Ok(headers)
}
