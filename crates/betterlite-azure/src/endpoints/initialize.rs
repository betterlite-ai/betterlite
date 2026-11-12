use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::azure_credentials::DEFAULT_SCOPE;
use betterlite_config::sections::azure_endpoints::Settings;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use crate::azure_urls::{
    AuthEnvironment, EndpointSelection, resolve_auth_environment, select_base_url_or_endpoint,
};
use crate::credentials::cache::RefreshProviderCache;
use crate::credentials::source::{CallableTokenProvider, CredentialSources, SourceFailure};
use crate::oidc_exchange::OidcExchangeInput;

use super::projection::{
    AzureClientInputs, CallableProvider, HttpClient, LiteLlmParams, SessionKind,
};

pub struct InitializeRequest<'data> {
    pub params: LiteLlmParams,
    pub provider: Option<CallableProvider>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub model_name: Option<String>,
    pub api_version: Option<String>,
    pub is_async: bool,
    pub session: SessionKind,
    pub refresh_enabled: bool,
    pub settings: &'data Settings,
    pub environment: &'data dyn EnvironmentLookup,
    pub sources: &'data dyn CredentialSources,
}

#[derive(Debug, Error)]
pub enum EndpointError {
    #[error(transparent)]
    Source(#[from] SourceFailure),
    #[error("the client custom query has no api-version")]
    MissingApiVersion,
}

impl EndpointError {
    #[must_use]
    pub const fn class(&self) -> &str {
        match self {
            Self::Source(failure) => failure.class.as_str(),
            Self::MissingApiVersion => "KeyError",
        }
    }

    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Source(failure) => failure.status,
            Self::MissingApiVersion => None,
        }
    }
}

pub fn initialize_azure_sdk_client<'data>(
    request: &InitializeRequest<'data>,
    refresh_cache: &mut RefreshProviderCache<'data>,
) -> Result<AzureClientInputs<'data>, EndpointError> {
    let environment = resolve_auth_environment(request.params.auth.clone(), request.environment);
    let scope = environment
        .azure_scope
        .clone()
        .unwrap_or_else(|| DEFAULT_SCOPE.to_owned());
    let (token, exchanged) = exchange_oidc_token(request, &environment, &scope)?;
    let mut provider = resolve_provider(request, &environment, &scope)?;
    let key_absent = request
        .api_key
        .as_ref()
        .is_none_or(|key| key.expose_secret().is_empty());
    if !exchanged && key_absent && provider.is_none() && request.refresh_enabled {
        provider = cached_refresh(refresh_cache, &scope)?;
    }
    let api_version = resolve_api_version(request);
    let endpoint = select_base_url_or_endpoint(EndpointSelection {
        azure_endpoint: request.api_base.clone(),
        base_url: None,
    });
    Ok(AzureClientInputs {
        endpoint,
        api_version,
        api_key: request.api_key.clone(),
        azure_ad_token: token,
        token_provider: provider,
        max_retries: request
            .params
            .max_retries
            .unwrap_or(request.settings.default_max_retries),
        timeout: request.params.timeout,
        http_client: HttpClient {
            is_async: request.is_async,
            shared: matches!(request.session, SessionKind::Shared),
        },
    })
}

// api_version is used unchanged unless it is None, then AZURE_API_VERSION, then the configured default
fn resolve_api_version(request: &InitializeRequest<'_>) -> String {
    request.api_version.as_deref().map_or_else(
        || {
            request
                .environment
                .lookup("AZURE_API_VERSION")
                .unwrap_or_else(|| request.settings.azure_default_api_version.clone())
        },
        str::to_owned,
    )
}

// LiteLLM runs the OIDC exchange whenever the configured token has its prefix, even next to a key or provider
fn exchange_oidc_token(
    request: &InitializeRequest<'_>,
    environment: &AuthEnvironment,
    scope: &str,
) -> Result<(Option<SecretString>, bool), EndpointError> {
    let Some(token) = request.params.azure_ad_token.as_ref() else {
        return Ok((None, false));
    };
    if !token.expose_secret().starts_with("oidc/") {
        return Ok((Some(token.clone()), false));
    }
    tracing::debug!("Using Azure OIDC Token for Azure Auth");
    let input = OidcExchangeInput {
        azure_ad_token: token.clone(),
        azure_client_id: environment.client_id.clone(),
        azure_tenant_id: environment.tenant_id.clone(),
        scope: Some(scope.to_owned()),
    };
    Ok((Some(request.sources.oidc(&input)?), true))
}

fn resolve_provider<'data>(
    request: &InitializeRequest<'data>,
    environment: &AuthEnvironment,
    scope: &str,
) -> Result<Option<Arc<dyn CallableTokenProvider + 'data>>, EndpointError> {
    if let Some(provider) = request.provider.as_ref() {
        return Ok(Some(Arc::clone(&provider.provider)));
    }
    let key_absent = request
        .api_key
        .as_ref()
        .is_none_or(|key| key.expose_secret().is_empty());
    if key_absent
        && let Some(provider) = client_secret_provider(environment, scope, request.sources)?
    {
        return Ok(Some(provider));
    }
    username_password_provider(environment, scope, request.sources)
}

fn client_secret_provider<'data>(
    environment: &AuthEnvironment,
    scope: &str,
    sources: &'data dyn CredentialSources,
) -> Result<Option<Arc<dyn CallableTokenProvider + 'data>>, EndpointError> {
    let (Some(tenant_id), Some(client_id), Some(client_secret)) = (
        environment.tenant_id.as_deref(),
        environment.client_id.as_deref(),
        environment.client_secret.as_ref(),
    ) else {
        return Ok(None);
    };
    if tenant_id.is_empty() || client_id.is_empty() || client_secret.expose_secret().is_empty() {
        return Ok(None);
    }
    tracing::debug!("Using Azure AD Token Provider from Entra ID for Azure Auth");
    let record = ClientSecretRecord {
        tenant_id: tenant_id.to_owned(),
        client_id: client_id.to_owned(),
        client_secret: client_secret.clone(),
        scope: scope.to_owned(),
    };
    Ok(Some(sources.client_secret(&record)?))
}

// username and password build their provider even when an API key is configured, as long as none was passed
fn username_password_provider<'data>(
    environment: &AuthEnvironment,
    scope: &str,
    sources: &'data dyn CredentialSources,
) -> Result<Option<Arc<dyn CallableTokenProvider + 'data>>, EndpointError> {
    let (Some(client_id), Some(username), Some(password)) = (
        environment.client_id.as_deref(),
        environment.azure_username.as_deref(),
        environment.azure_password.as_ref(),
    ) else {
        return Ok(None);
    };
    if client_id.is_empty() || username.is_empty() || password.expose_secret().is_empty() {
        return Ok(None);
    }
    tracing::debug!("Using Azure Username and Password for Azure Auth");
    let record = UsernamePasswordRecord {
        client_id: client_id.to_owned(),
        username: username.to_owned(),
        password: password.clone(),
        tenant_id: None,
        scope: scope.to_owned(),
    };
    Ok(Some(sources.username_password(&record)?))
}

// only a ValueError from the cached refresh provider is swallowed; every other failure propagates
fn cached_refresh<'data>(
    refresh_cache: &mut RefreshProviderCache<'data>,
    scope: &str,
) -> Result<Option<Arc<dyn CallableTokenProvider + 'data>>, EndpointError> {
    match refresh_cache.provider(scope) {
        Ok(provider) => Ok(Some(provider)),
        Err(failure) if failure.class == "ValueError" => {
            tracing::debug!("Azure AD Token Provider could not be used.");
            Ok(None)
        }
        Err(failure) => {
            tracing::error!("Error calling Azure AD token provider: {}", failure.message);
            Err(EndpointError::Source(failure))
        }
    }
}
