use std::collections::BTreeMap;

use betterlite_config::sso_mappings::{
    GENERIC_AUTHORIZATION_ENDPOINT_ENV, GENERIC_CLIENT_SECRET_ENV, GENERIC_TOKEN_ENDPOINT_ENV,
    GENERIC_USERINFO_ENDPOINT_ENV, SsoMappingSources,
};
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use secrecy::SecretString;
use thiserror::Error;

pub const DEFAULT_GENERIC_SCOPE: &str = "openid email profile";

// The four values the generic OAuth flow sends to the provider, resolved once at setup time.
#[derive(Debug, Clone)]
pub struct GenericSsoSettings {
    pub client_secret: SecretString,
    pub scope: Vec<String>,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub userinfo_endpoint: String,
    pub include_client_id: bool,
}

// LiteLLM raises ProxyException itself here, so the boxed wire error is the whole failure
#[derive(Debug, Error)]
#[error(transparent)]
pub struct MissingVariable(pub Box<ProxyException>);

pub fn generic_sso_settings(
    sources: &SsoMappingSources,
) -> Result<GenericSsoSettings, MissingVariable> {
    Ok(GenericSsoSettings {
        client_secret: SecretString::from(require(
            sources.client_secret.as_deref(),
            GENERIC_CLIENT_SECRET_ENV,
        )?),
        scope: sources
            .scope
            .as_deref()
            .unwrap_or(DEFAULT_GENERIC_SCOPE)
            .split(' ')
            .map(str::to_owned)
            .collect(),
        authorization_endpoint: require(
            sources.authorization_endpoint.as_deref(),
            GENERIC_AUTHORIZATION_ENDPOINT_ENV,
        )?,
        token_endpoint: require(
            sources.token_endpoint.as_deref(),
            GENERIC_TOKEN_ENDPOINT_ENV,
        )?,
        userinfo_endpoint: require(
            sources.userinfo_endpoint.as_deref(),
            GENERIC_USERINFO_ENDPOINT_ENV,
        )?,
        include_client_id: sources.include_client_id,
    })
}

// LiteLLM refuses an unset value only; an explicitly empty secret is accepted
fn require(value: Option<&str>, variable: &'static str) -> Result<String, MissingVariable> {
    let Some(text) = value else {
        return Err(MissingVariable(Box::new(ProxyException::new(
            ProxyExceptionInit {
                message: format!("{variable} not set. Set it in .env file"),
                error_type: "auth_error".to_owned(),
                param: Some(variable.to_owned()),
                code: ExceptionCode::Status(500),
                headers: BTreeMap::new(),
                openai_code: None,
                provider_specific_fields: BTreeMap::new(),
            },
        ))));
    };
    Ok(text.to_owned())
}
