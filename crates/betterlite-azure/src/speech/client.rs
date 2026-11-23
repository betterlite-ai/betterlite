use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::azure_endpoints::Settings;
use betterlite_http::user_url::httpx_url::HttpxUrl;
use secrecy::{ExposeSecret as _, SecretString};

use crate::azure_urls::{EndpointSelection, select_base_url_or_endpoint};
use crate::credentials::cache::RefreshProviderCache;
use crate::credentials::source::{CredentialSources, TokenReply};
use crate::endpoints::get_client::{ClientCache, GetClientRequest, get_azure_openai_client};
use crate::endpoints::projection::{
    AzureClientInputs, CallableProvider, ClientInputs, ClientKind, ClientOutcome, LiteLlmParams,
    OpenAiV1ClientInputs, SessionKind, SuppliedClient, TokenProvider, V1Key,
};

use super::error::AzureSpeechError;

pub struct ClientWiring<'cache, 'data> {
    pub settings: &'data Settings,
    pub environment: &'data dyn EnvironmentLookup,
    pub sources: &'data dyn CredentialSources,
    pub cache: &'cache mut dyn ClientCache,
    pub refresh: &'cache mut RefreshProviderCache<'data>,
}

// The arguments audio_speech and async_audio_speech hand get_azure_openai_client.
pub struct SpeechConnection {
    pub model: String,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub params: LiteLlmParams,
    pub provider: Option<CallableProvider>,
    pub supplied: Option<SuppliedClient>,
    pub session: SessionKind,
    pub refresh_enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientFamily {
    Azure,
    OpenAiV1,
}

pub enum RequestAuth {
    ApiKey(SecretString),
    Bearer(SecretString),
}

// what the request builder reads off the client the SDK would have constructed
pub struct SelectedSpeechClient<'data> {
    pub family: ClientFamily,
    pub base_url: String,
    pub api_version: Option<String>,
    pub api_key: Option<SecretString>,
    pub ad_token: Option<SecretString>,
    pub provider: Option<TokenProvider<'data>>,
    pub v1_key: Option<V1Key<'data>>,
}

pub fn select_client<'data>(
    connection: &SpeechConnection,
    is_async: bool,
    wiring: &mut ClientWiring<'_, 'data>,
) -> Result<SelectedSpeechClient<'data>, AzureSpeechError> {
    let outcome = get_azure_openai_client(
        &client_request(connection, is_async, wiring),
        wiring.cache,
        wiring.refresh,
    )?;
    selected_client(outcome, connection, wiring, is_async)
}

fn client_request<'data>(
    connection: &SpeechConnection,
    is_async: bool,
    wiring: &ClientWiring<'_, 'data>,
) -> GetClientRequest<'data> {
    GetClientRequest {
        api_key: connection.api_key.clone(),
        api_base: connection.api_base.clone(),
        api_version: connection.api_version.clone(),
        model: Some(connection.model.clone()),
        is_async,
        params: connection.params.clone(),
        provider: connection.provider.clone(),
        supplied: connection.supplied.clone(),
        settings: wiring.settings,
        environment: wiring.environment,
        refresh_enabled: connection.refresh_enabled,
        session: connection.session,
        sources: wiring.sources,
    }
}

fn selected_client<'data>(
    outcome: ClientOutcome<'data>,
    connection: &SpeechConnection,
    wiring: &ClientWiring<'_, 'data>,
    is_async: bool,
) -> Result<SelectedSpeechClient<'data>, AzureSpeechError> {
    let ClientOutcome {
        kind,
        inputs,
        custom_query,
        ..
    } = outcome;
    let v1 = match kind {
        ClientKind::AzureOpenAi | ClientKind::AsyncAzureOpenAi => false,
        ClientKind::OpenAi | ClientKind::AsyncOpenAi => true,
        ClientKind::Other => return Err(AzureSpeechError::UnsupportedClient),
    };
    match inputs {
        Some(ClientInputs::Azure(azure)) if !v1 => from_azure_inputs(azure),
        Some(ClientInputs::OpenAiV1(keys)) if v1 => from_v1_inputs(keys),
        _ if v1 => v1_fallback(connection, is_async),
        _ => azure_fallback(connection, custom_query.as_ref(), wiring),
    }
}

fn from_azure_inputs<'data>(
    azure: AzureClientInputs<'data>,
) -> Result<SelectedSpeechClient<'data>, AzureSpeechError> {
    Ok(SelectedSpeechClient {
        family: ClientFamily::Azure,
        base_url: normalized_base_url(&azure.endpoint)?,
        api_version: Some(azure.api_version),
        api_key: azure.api_key,
        ad_token: azure.azure_ad_token,
        provider: azure.token_provider,
        v1_key: None,
    })
}

fn from_v1_inputs<'data>(
    keys: OpenAiV1ClientInputs<'data>,
) -> Result<SelectedSpeechClient<'data>, AzureSpeechError> {
    Ok(SelectedSpeechClient {
        family: ClientFamily::OpenAiV1,
        base_url: with_trailing_slash(&keys.base_url.ok_or(AzureSpeechError::MissingBase)?)?,
        api_version: None,
        api_key: None,
        ad_token: None,
        provider: None,
        v1_key: keys.key,
    })
}

// a supplied or cached client carries no inputs: the call's own values reproduce its base and version
fn azure_fallback<'data>(
    connection: &SpeechConnection,
    custom_query: Option<&BTreeMap<String, String>>,
    wiring: &ClientWiring<'_, 'data>,
) -> Result<SelectedSpeechClient<'data>, AzureSpeechError> {
    let selection = select_base_url_or_endpoint(EndpointSelection {
        azure_endpoint: connection.api_base.clone(),
        base_url: None,
    });
    let version = custom_query
        .and_then(|query| query.get("api-version").cloned())
        .or_else(|| connection.api_version.clone())
        .or_else(|| wiring.environment.lookup("AZURE_API_VERSION"))
        .unwrap_or_else(|| wiring.settings.azure_default_api_version.clone());
    Ok(SelectedSpeechClient {
        family: ClientFamily::Azure,
        base_url: normalized_base_url(&selection)?,
        api_version: Some(version),
        api_key: connection.api_key.clone(),
        ad_token: connection.params.azure_ad_token.clone(),
        provider: connection
            .provider
            .as_ref()
            .map(|provider| TokenProvider(Arc::clone(&provider.provider))),
        v1_key: None,
    })
}

// LiteLLM's v1 client takes "{api_base}/openai/v1/" and the first present of api_key, the provider and the token
fn v1_fallback<'data>(
    connection: &SpeechConnection,
    is_async: bool,
) -> Result<SelectedSpeechClient<'data>, AzureSpeechError> {
    let api_base = connection
        .api_base
        .as_deref()
        .ok_or(AzureSpeechError::MissingBase)?;
    let key = connection.api_key.clone().map(V1Key::Static).or_else(|| {
        connection
            .provider
            .as_ref()
            .map(|provider| V1Key::Provider {
                provider: TokenProvider(Arc::clone(&provider.provider)),
                offload: is_async,
            })
    });
    Ok(SelectedSpeechClient {
        family: ClientFamily::OpenAiV1,
        base_url: with_trailing_slash(&format!("{api_base}/openai/v1/"))?,
        api_version: None,
        api_key: None,
        ad_token: None,
        provider: None,
        v1_key: key.or_else(|| connection.params.azure_ad_token.clone().map(V1Key::Static)),
    })
}

// AzureOpenAI builds "{azure_endpoint}/openai", or takes base_url as given, and enforces a trailing slash
fn normalized_base_url(selection: &EndpointSelection) -> Result<String, AzureSpeechError> {
    let raw = match (&selection.base_url, &selection.azure_endpoint) {
        (Some(base), _) => base.clone(),
        (None, Some(endpoint)) => format!("{}/openai", endpoint.trim_end_matches('/')),
        (None, None) => return Err(AzureSpeechError::MissingBase),
    };
    with_trailing_slash(&raw)
}

fn with_trailing_slash(raw: &str) -> Result<String, AzureSpeechError> {
    let mut parsed = HttpxUrl::parse(raw).map_err(|source| AzureSpeechError::Url { source })?;
    if !parsed.path.ends_with('/') {
        parsed.path.push('/');
    }
    Ok(parsed.href())
}

// AzureOpenAI sends the token first, then the key, and keeps an Authorization header the caller set
pub fn request_auth(
    ad_token: Option<SecretString>,
    provider: Option<&TokenProvider<'_>>,
    api_key: Option<SecretString>,
) -> Result<RequestAuth, AzureSpeechError> {
    if let Some(token) = ad_token {
        return Ok(RequestAuth::Bearer(token));
    }
    if let Some(callable) = provider {
        return Ok(RequestAuth::Bearer(provider_token(callable)?));
    }
    api_key
        .filter(|key| !key.expose_secret().is_empty())
        .map(RequestAuth::ApiKey)
        .ok_or(AzureSpeechError::UnableToHandleAuth)
}

pub fn v1_auth(key: Option<&V1Key<'_>>) -> Result<RequestAuth, AzureSpeechError> {
    match key {
        Some(V1Key::Static(key)) => Ok(RequestAuth::Bearer(key.clone())),
        Some(V1Key::Provider { provider, .. }) => {
            Ok(RequestAuth::Bearer(provider_token(provider)?))
        }
        None => Err(AzureSpeechError::MissingKey),
    }
}

fn provider_token(provider: &TokenProvider<'_>) -> Result<SecretString, AzureSpeechError> {
    let token = provider
        .0
        .token()
        .map_err(|failure| AzureSpeechError::TokenProvider {
            class: failure.class,
            message: failure.message,
        })?;
    match token {
        TokenReply::Text(text) => Ok(text),
        TokenReply::NonText { type_name } => Err(AzureSpeechError::TokenNotString {
            returned: type_name,
        }),
    }
}
