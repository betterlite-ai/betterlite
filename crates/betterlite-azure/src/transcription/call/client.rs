use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::azure_endpoints::Settings;
use betterlite_http::client_kind::ClientKind as CallKind;
use betterlite_http::user_url::httpx_url::HttpxUrl;
use secrecy::{ExposeSecret, SecretString};

use crate::azure_urls::{EndpointSelection, select_base_url_or_endpoint};
use crate::credentials::cache::RefreshProviderCache;
use crate::credentials::source::{CredentialSources, TokenReply};
use crate::endpoints::get_client::{ClientCache, GetClientRequest, get_azure_openai_client};
use crate::endpoints::projection::{
    CallableProvider, ClientInputs, ClientKind, LiteLlmParams, SessionKind, SuppliedClient,
    TokenProvider,
};

use super::AzureTranscriptionError;

// openai's AzureOpenAI fills a missing key with this sentinel, which LiteLLM then logs as the client key
const SENTINEL: &str = "<missing API key>";

pub struct ClientWiring<'cache, 'data> {
    pub settings: &'data Settings,
    pub environment: &'data dyn EnvironmentLookup,
    pub sources: &'data dyn CredentialSources,
    pub cache: &'cache mut dyn ClientCache,
    pub refresh: &'cache mut RefreshProviderCache<'data>,
}

// The arguments OpenAIAudioTranscription.audio_transcriptions hands get_azure_openai_client.
pub struct Connection {
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

pub enum RequestAuth {
    ApiKey(SecretString),
    Bearer(SecretString),
}

// what the request builder and the logging hooks read off the selected client; the SDK picks its auth header after pre_call.
pub struct SelectedClient<'data> {
    pub base_url: String,
    pub api_version: Option<String>,
    // "Bearer {azure_client.api_key}", the value pre_call and the async post_call log
    pub authorization: String,
    pub ad_token: Option<SecretString>,
    pub provider: Option<TokenProvider<'data>>,
    pub api_key: Option<SecretString>,
}

pub fn select_client<'data>(
    connection: &Connection,
    kind: CallKind,
    wiring: &mut ClientWiring<'_, 'data>,
) -> Result<SelectedClient<'data>, AzureTranscriptionError> {
    let is_async = kind == CallKind::Asynchronous;
    let request = client_request(connection, is_async, wiring);
    let outcome = get_azure_openai_client(&request, wiring.cache, wiring.refresh)?;
    let accepted = match kind {
        CallKind::Sync => outcome.kind == ClientKind::AzureOpenAi,
        CallKind::Asynchronous => outcome.kind == ClientKind::AsyncAzureOpenAi,
    };
    if !accepted {
        return Err(not_azure(is_async));
    }
    let material = client_material(
        outcome.inputs,
        connection,
        outcome.custom_query.as_ref(),
        wiring,
    );
    let authorization = format!(
        "Bearer {}",
        material
            .api_key
            .as_ref()
            .map_or(SENTINEL, ExposeSecret::expose_secret)
    );
    Ok(SelectedClient {
        base_url: normalized_base_url(&material.selection)?,
        api_version: material.api_version,
        authorization,
        ad_token: material.ad_token,
        provider: material.provider,
        api_key: material.api_key,
    })
}

fn client_request<'data>(
    connection: &Connection,
    is_async: bool,
    wiring: &ClientWiring<'_, 'data>,
) -> GetClientRequest<'data> {
    GetClientRequest {
        api_key: connection.api_key.clone(),
        api_base: connection.api_base.clone(),
        // the atranscription branch never forwards its api_version to the async helper
        api_version: if is_async {
            None
        } else {
            connection.api_version.clone()
        },
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

const fn not_azure(is_async: bool) -> AzureTranscriptionError {
    if is_async {
        AzureTranscriptionError::NotAsyncAzureClient
    } else {
        AzureTranscriptionError::NotAzureClient
    }
}

struct ClientMaterial<'data> {
    selection: EndpointSelection,
    api_version: Option<String>,
    api_key: Option<SecretString>,
    ad_token: Option<SecretString>,
    provider: Option<TokenProvider<'data>>,
}

// a constructed client carries its inputs; a supplied or cached one falls back to the call's own
fn client_material<'data>(
    inputs: Option<ClientInputs<'data>>,
    connection: &Connection,
    custom_query: Option<&BTreeMap<String, String>>,
    wiring: &ClientWiring<'_, 'data>,
) -> ClientMaterial<'data> {
    if let Some(ClientInputs::Azure(azure)) = inputs {
        return ClientMaterial {
            selection: azure.endpoint,
            api_version: Some(azure.api_version),
            api_key: azure.api_key,
            ad_token: azure.azure_ad_token,
            provider: azure.token_provider,
        };
    }
    // the supplied client keeps its own api-version, and a cached client falls back to the version a construction resolves
    let api_version = custom_query.map_or_else(
        || {
            let configured = connection
                .api_version
                .clone()
                .or_else(|| wiring.environment.lookup("AZURE_API_VERSION"));
            Some(configured.unwrap_or_else(|| wiring.settings.azure_default_api_version.clone()))
        },
        |query| query.get("api-version").cloned(),
    );
    ClientMaterial {
        selection: select_base_url_or_endpoint(EndpointSelection {
            azure_endpoint: connection.api_base.clone(),
            base_url: None,
        }),
        api_version,
        api_key: connection.api_key.clone(),
        ad_token: connection.params.azure_ad_token.clone(),
        provider: connection
            .provider
            .as_ref()
            .map(|provider| TokenProvider(Arc::clone(&provider.provider))),
    }
}

pub fn request_auth(
    ad_token: Option<SecretString>,
    provider: Option<&TokenProvider<'_>>,
    api_key: Option<SecretString>,
) -> Result<RequestAuth, AzureTranscriptionError> {
    if let Some(token) = ad_token {
        return Ok(RequestAuth::Bearer(token));
    }
    if let Some(callable) = provider {
        let token =
            callable
                .0
                .token()
                .map_err(|failure| AzureTranscriptionError::TokenProvider {
                    class: failure.class,
                    message: failure.message,
                })?;
        return match token {
            TokenReply::Text(text) if !text.expose_secret().is_empty() => {
                Ok(RequestAuth::Bearer(text))
            }
            TokenReply::Text(_) => Err(AzureTranscriptionError::TokenNotString {
                returned: String::new(),
            }),
            TokenReply::NonText { type_name } => Err(AzureTranscriptionError::TokenNotString {
                returned: type_name,
            }),
        };
    }
    api_key
        .filter(|key| !key.expose_secret().is_empty())
        .map(RequestAuth::ApiKey)
        .ok_or(AzureTranscriptionError::UnableToHandleAuth)
}

// AzureOpenAI builds "{azure_endpoint}/openai", or takes base_url as given, and enforces a trailing slash
fn normalized_base_url(selection: &EndpointSelection) -> Result<String, AzureTranscriptionError> {
    let raw = match (&selection.base_url, &selection.azure_endpoint) {
        (Some(base), _) => base.clone(),
        (None, Some(endpoint)) => format!("{}/openai", endpoint.trim_end_matches('/')),
        (None, None) => return Err(AzureTranscriptionError::MissingBase),
    };
    let mut parsed =
        HttpxUrl::parse(&raw).map_err(|source| AzureTranscriptionError::Url { source })?;
    if !parsed.path.ends_with('/') {
        parsed.path.push('/');
    }
    Ok(parsed.href())
}
