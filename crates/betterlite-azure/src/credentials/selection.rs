use std::sync::Arc;

use betterlite_config::sections::azure_credentials::DeploymentSettings;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use crate::oidc_exchange::OidcExchangeInput;

use super::source::{CallableTokenProvider, CredentialSources, SourceFailure, TokenReply};

const OIDC_PREFIX: &str = "oidc/";
const VALUE_ERROR: &str = "ValueError";

pub struct AzureCredentialRequest<'data> {
    pub params: &'data DeploymentSettings,
    pub refresh_enabled: bool,
    pub provider: Option<Arc<dyn CallableTokenProvider + 'data>>,
    pub sources: &'data dyn CredentialSources,
}

#[derive(Debug, Error)]
pub enum AzureTokenError {
    #[error("Azure AD token must be a string, got {type_name}")]
    TokenNotString { type_name: String },
    #[error("{message}")]
    ProviderTypeError { message: String },
    #[error("Failed to get Azure AD token: {message}")]
    ProviderFailed { message: String },
    #[error("{message}")]
    Source {
        class: String,
        status: Option<u16>,
        message: String,
    },
}

impl AzureTokenError {
    #[must_use]
    pub fn class(&self) -> &str {
        match self {
            Self::TokenNotString { .. } | Self::ProviderTypeError { .. } => "TypeError",
            Self::ProviderFailed { .. } => "RuntimeError",
            Self::Source { class, .. } => class,
        }
    }

    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Source { status, .. } => *status,
            Self::TokenNotString { .. }
            | Self::ProviderTypeError { .. }
            | Self::ProviderFailed { .. } => None,
        }
    }
}

impl From<SourceFailure> for AzureTokenError {
    fn from(failure: SourceFailure) -> Self {
        Self::Source {
            class: failure.class,
            status: failure.status,
            message: failure.message,
        }
    }
}

pub fn get_azure_ad_token<'request>(
    request: &'request AzureCredentialRequest<'_>,
) -> Result<Option<SecretString>, AzureTokenError> {
    let mut token = request.params.azure_ad_token.clone();
    let mut provider: Option<Arc<dyn CallableTokenProvider + 'request>> = request.provider.clone();
    if provider.is_none() {
        provider = resolve_provider(request.params, request.sources)?;
    }
    if !apply_oidc(request, &mut token)? && provider.is_none() && request.refresh_enabled {
        apply_refresh(request, token.as_ref(), &mut provider)?;
    }
    provider.map_or(Ok(token), |selected| {
        invoke_token(selected.as_ref()).map(Some)
    })
}

fn resolve_provider<'request>(
    params: &DeploymentSettings,
    sources: &'request dyn CredentialSources,
) -> Result<Option<Arc<dyn CallableTokenProvider + 'request>>, AzureTokenError> {
    let client_id = params.client_id.as_deref().filter(|text| !text.is_empty());
    let identity = (
        params.tenant_id.as_deref().filter(|text| !text.is_empty()),
        client_id,
        params
            .client_secret
            .as_ref()
            .filter(|secret| !secret.expose_secret().is_empty()),
    );
    if let (Some(tenant), Some(client), Some(secret)) = identity {
        tracing::debug!("Using Azure AD Token Provider from Entra ID for Azure Auth");
        let record = ClientSecretRecord {
            tenant_id: tenant.to_owned(),
            client_id: client.to_owned(),
            client_secret: secret.clone(),
            scope: params.azure_scope.clone(),
        };
        return sources
            .client_secret(&record)
            .map(Some)
            .map_err(AzureTokenError::from);
    }
    username_provider(params, client_id, sources)
}

fn username_provider<'request>(
    params: &DeploymentSettings,
    client_id: Option<&str>,
    sources: &'request dyn CredentialSources,
) -> Result<Option<Arc<dyn CallableTokenProvider + 'request>>, AzureTokenError> {
    let login = (
        params
            .azure_username
            .as_deref()
            .filter(|text| !text.is_empty()),
        params
            .azure_password
            .as_ref()
            .filter(|secret| !secret.expose_secret().is_empty()),
        client_id,
    );
    let (Some(user), Some(pass), Some(client)) = login else {
        return Ok(None);
    };
    tracing::debug!("Using Azure Username and Password for Azure Auth");
    // the username and password credential discovers its tenant from the account realm
    let record = UsernamePasswordRecord {
        client_id: client.to_owned(),
        username: user.to_owned(),
        password: pass.clone(),
        tenant_id: None,
        scope: params.azure_scope.clone(),
    };
    sources
        .username_password(&record)
        .map(Some)
        .map_err(AzureTokenError::from)
}

fn apply_oidc(
    request: &AzureCredentialRequest<'_>,
    token: &mut Option<SecretString>,
) -> Result<bool, AzureTokenError> {
    // LiteLLM decides on the token value it has after the two provider branches
    let configured = token
        .as_ref()
        .filter(|value| value.expose_secret().starts_with(OIDC_PREFIX))
        .cloned();
    let identity = (
        configured,
        request
            .params
            .client_id
            .as_deref()
            .filter(|text| !text.is_empty()),
        request
            .params
            .tenant_id
            .as_deref()
            .filter(|text| !text.is_empty()),
    );
    let (Some(assertion), Some(client_id), Some(tenant_id)) = identity else {
        return Ok(false);
    };
    tracing::debug!("Using Azure OIDC Token for Azure Auth");
    let input = OidcExchangeInput {
        azure_ad_token: assertion,
        azure_client_id: Some(client_id.to_owned()),
        azure_tenant_id: Some(tenant_id.to_owned()),
        scope: Some(request.params.azure_scope.clone()),
    };
    *token = Some(request.sources.oidc(&input)?);
    Ok(true)
}

fn apply_refresh<'request>(
    request: &'request AzureCredentialRequest<'_>,
    token: Option<&SecretString>,
    provider: &mut Option<Arc<dyn CallableTokenProvider + 'request>>,
) -> Result<(), AzureTokenError> {
    tracing::debug!(
        "Using Azure AD token provider based on Service Principal with Secret workflow or DefaultAzureCredential for Azure Auth"
    );
    match request.sources.sdk_refresh(&request.params.azure_scope) {
        Ok(refresh) => *provider = Some(refresh),
        Err(failure) if failure.class == VALUE_ERROR => {
            tracing::debug!("Azure AD Token Provider could not be used.");
        }
        Err(failure) => {
            tracing::error!(
                "Error calling Azure AD token provider: {}. Follow docs - https://docs.litellm.ai/docs/providers/azure/#azure-ad-token-refresh---defaultazurecredential",
                failure.message
            );
            return Err(AzureTokenError::from(failure));
        }
    }
    if provider.is_none() && token.is_none() {
        *provider = request
            .sources
            .default_credential(request.params.azure_scope.as_str());
    }
    Ok(())
}

fn invoke_token(provider: &dyn CallableTokenProvider) -> Result<SecretString, AzureTokenError> {
    match provider.token() {
        Ok(TokenReply::Text(text)) => Ok(text),
        Ok(TokenReply::NonText { type_name }) => Err(AzureTokenError::TokenNotString { type_name }),
        Err(failure) if failure.class == "TypeError" => Err(AzureTokenError::ProviderTypeError {
            message: failure.message,
        }),
        Err(failure) => Err(AzureTokenError::ProviderFailed {
            message: failure.message,
        }),
    }
}
