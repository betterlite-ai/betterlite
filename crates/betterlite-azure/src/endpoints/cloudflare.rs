use std::fmt;
use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::azure_credentials::DEFAULT_SCOPE;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::credentials::source::{CallableTokenProvider, CredentialSources};
use crate::oidc_exchange::OidcExchangeInput;

use super::initialize::EndpointError;
use super::projection::{ClientKind, SuppliedClient};

// litellm_params distinguishes an absent key (None) from an explicit null, because dict.get keeps null
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum ParamValue<T> {
    Null,
    Value(T),
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloudflareParams {
    #[serde(default)]
    pub tenant_id: Option<ParamValue<String>>,
    #[serde(default)]
    pub client_id: Option<ParamValue<String>>,
    #[serde(default)]
    pub azure_scope: Option<ParamValue<String>>,
}

pub struct CloudflareRequest<'data> {
    pub api_base: &'data str,
    pub model: &'data str,
    pub api_version: &'data str,
    pub max_retries: i64,
    pub timeout: f64,
    pub params: &'data CloudflareParams,
    pub api_key: Option<&'data SecretString>,
    pub azure_ad_token: Option<&'data SecretString>,
    pub token_provider: Option<&'data Arc<dyn CallableTokenProvider + 'data>>,
    pub shared_session: bool,
    pub is_async: bool,
    pub client: Option<&'data SuppliedClient>,
    pub environment: &'data dyn EnvironmentLookup,
    pub sources: &'data dyn CredentialSources,
}

pub struct CloudflareClient<'data> {
    pub base_url: String,
    pub api_version: String,
    pub api_key: Option<SecretString>,
    pub azure_ad_token: Option<SecretString>,
    pub token_provider: Option<Arc<dyn CallableTokenProvider + 'data>>,
    pub max_retries: i64,
    pub timeout: f64,
    pub shared_session: bool,
    pub kind: ClientKind,
}

impl fmt::Debug for CloudflareClient<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CloudflareClient")
            .field("base_url", &self.base_url)
            .field("api_version", &self.api_version)
            .field("api_key", &self.api_key)
            .field("azure_ad_token", &self.azure_ad_token)
            .field("token_provider", &self.token_provider.is_some())
            .field("max_retries", &self.max_retries)
            .field("timeout", &self.timeout)
            .field("shared_session", &self.shared_session)
            .field("kind", &self.kind)
            .finish()
    }
}

#[derive(Debug)]
pub enum CloudflareOutcome<'data> {
    Supplied(SuppliedClient),
    Constructed(CloudflareClient<'data>),
}

pub fn init_cloudflare_gateway_client<'data>(
    request: &CloudflareRequest<'data>,
) -> Result<CloudflareOutcome<'data>, EndpointError> {
    let tenant_id = dict_get(
        request.params.tenant_id.as_ref(),
        request.environment,
        "AZURE_TENANT_ID",
    );
    let client_id = dict_get(
        request.params.client_id.as_ref(),
        request.environment,
        "AZURE_CLIENT_ID",
    );
    let scope = resolve_scope(request);
    let Some(client) = request.client else {
        return Ok(CloudflareOutcome::Constructed(gateway_client(
            request, tenant_id, client_id, scope,
        )?));
    };
    Ok(CloudflareOutcome::Supplied(SuppliedClient {
        kind: client.kind,
        custom_query: client.custom_query.clone(),
    }))
}

// os.getenv's default is evaluated only for an absent key, so an explicit null scope stays None
fn resolve_scope(request: &CloudflareRequest<'_>) -> Option<String> {
    match &request.params.azure_scope {
        None => Some(
            request
                .environment
                .lookup("AZURE_SCOPE")
                .unwrap_or_else(|| DEFAULT_SCOPE.to_owned()),
        ),
        Some(ParamValue::Null) => None,
        Some(ParamValue::Value(text)) => Some(text.clone()),
    }
}

fn gateway_client<'data>(
    request: &CloudflareRequest<'data>,
    tenant_id: Option<String>,
    client_id: Option<String>,
    scope: Option<String>,
) -> Result<CloudflareClient<'data>, EndpointError> {
    let mut base_url = request.api_base.to_owned();
    if !base_url.ends_with('/') {
        base_url.push('/');
    }
    base_url.push_str(request.model);
    Ok(CloudflareClient {
        base_url,
        api_version: request.api_version.to_owned(),
        api_key: request.api_key.cloned(),
        azure_ad_token: exchanged_token(request, tenant_id, client_id, scope)?,
        token_provider: request.token_provider.cloned(),
        max_retries: request.max_retries,
        timeout: request.timeout,
        shared_session: request.shared_session,
        kind: if request.is_async {
            ClientKind::AsyncAzureOpenAi
        } else {
            ClientKind::AzureOpenAi
        },
    })
}

// an api key wins over the token entirely; the token keeps a non-oidc value and is exchanged otherwise
fn exchanged_token(
    request: &CloudflareRequest<'_>,
    tenant_id: Option<String>,
    client_id: Option<String>,
    scope: Option<String>,
) -> Result<Option<SecretString>, EndpointError> {
    let Some(token) = request.azure_ad_token.filter(|_| request.api_key.is_none()) else {
        return Ok(None);
    };
    if !token.expose_secret().starts_with("oidc/") {
        return Ok(Some(token.clone()));
    }
    let input = OidcExchangeInput {
        azure_ad_token: token.clone(),
        azure_client_id: client_id,
        azure_tenant_id: tenant_id,
        scope,
    };
    Ok(Some(request.sources.oidc(&input)?))
}

// dict.get(key, os.getenv(...)) keeps an explicit null, so only an absent key reads the environment
fn dict_get(
    value: Option<&ParamValue<String>>,
    environment: &dyn EnvironmentLookup,
    name: &str,
) -> Option<String> {
    match value {
        None => environment.lookup(name),
        Some(ParamValue::Null) => None,
        Some(ParamValue::Value(text)) => Some(text.clone()),
    }
}
