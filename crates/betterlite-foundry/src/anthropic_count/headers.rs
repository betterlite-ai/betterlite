use std::convert::Infallible;

use betterlite_azure::credentials::selection::AzureTokenError;
use secrecy::{ExposeSecret as _, SecretString};

use crate::anthropic_messages::environment::{
    EnvironmentRequest, Headers, ParamsKey, TokenSource, validate_environment,
};
use crate::anthropic_messages::environment_error::EnvironmentError;

const ANTHROPIC_BETA_HEADER: &str = "anthropic-beta";
const ANTHROPIC_KEY_HEADER: &str = "x-api-key";

pub struct HeadersRequest<'call> {
    pub api_key: &'call SecretString,
    pub params_key: ParamsKey<'call>,
    pub configured_key: Option<&'call SecretString>,
    pub token: &'call TokenSource<'call>,
}

// The Foundry messages environment already adds content-type and anthropic-version and merges the Azure auth
pub fn get_required_headers(
    request: &HeadersRequest<'_>,
    beta_version: &str,
) -> Result<Headers, AzureTokenError> {
    let mut key_headers = Headers::new();
    key_headers.insert(
        ANTHROPIC_KEY_HEADER.to_owned(),
        request.api_key.expose_secret().to_owned(),
    );
    let environment = EnvironmentRequest {
        headers: key_headers,
        params_key: request.params_key,
        argument_key: Some(request.api_key),
        configured_key: request.configured_key,
        token: TokenSource {
            credentials: request.token.credentials,
            refresh_enabled: request.token.refresh_enabled,
            provider: request.token.provider.clone(),
            sources: request.token.sources,
        },
        api_base: None,
    };
    let (headers, _api_base) = validate_environment(environment, |mut with_key| {
        with_key.insert(ANTHROPIC_BETA_HEADER.to_owned(), beta_version.to_owned());
        Ok::<Headers, Infallible>(with_key)
    })
    .map_err(|error| match error {
        EnvironmentError::Token(source) => source,
        EnvironmentError::Betas(never) => match never {},
    })?;
    Ok(headers)
}
