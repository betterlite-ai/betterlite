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
    CallableProvider, ClientInputs, ClientKind, ClientOutcome, LiteLlmParams, SessionKind,
    SuppliedClient, TokenProvider, V1Key,
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
    pub aspeech: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Azure,
    OpenAiV1,
}

// The credential the client sends, before any header the caller already set is looked at.
pub enum Key<'data> {
    ApiKey(SecretString),
    Bearer(SecretString),
    Provider(TokenProvider<'data>),
    Absent,
}

// The selected SDK client the request builder reads: its base, its version and the credential it would send.
pub struct Selected<'data> {
    pub family: Family,
    pub base_url: String,
    pub api_version: Option<String>,
    pub auth: Key<'data>,
}

pub enum RequestAuth {
    ApiKey(SecretString),
    Bearer(SecretString),
}

pub fn select_client<'data>(
    connection: &SpeechConnection,
    wiring: &mut ClientWiring<'_, 'data>,
) -> Result<Selected<'data>, AzureSpeechError> {
    let request = client_request(connection, wiring);
    let outcome = get_azure_openai_client(&request, wiring.cache, wiring.refresh)?;
    let ClientOutcome {
        kind,
        inputs,
        custom_query,
        ..
    } = outcome;
    if kind == ClientKind::Other {
        return Err(AzureSpeechError::UnsupportedClient);
    }
    let v1 = matches!(kind, ClientKind::OpenAi | ClientKind::AsyncOpenAi);
    inputs.map_or_else(
        || {
            if v1 {
                v1_fallback(connection)
            } else {
                azure_fallback(connection, custom_query.as_ref(), wiring)
            }
        },
        selected_from_inputs,
    )
}

fn selected_from_inputs(inputs: ClientInputs<'_>) -> Result<Selected<'_>, AzureSpeechError> {
    match inputs {
        ClientInputs::Azure(azure) => {
            let base_url = normalized_base_url(&azure.endpoint)?;
            let auth = azure_key(azure.azure_ad_token, azure.token_provider, azure.api_key);
            Ok(Selected {
                family: Family::Azure,
                base_url,
                api_version: Some(azure.api_version),
                auth,
            })
        }
        ClientInputs::OpenAiV1(keys) => {
            let Some(base_url) = keys.base_url else {
                return Err(AzureSpeechError::MissingBase);
            };
            let auth = v1_key(keys.key);
            Ok(Selected {
                family: Family::OpenAiV1,
                base_url: with_trailing_slash(&base_url)?,
                api_version: None,
                auth,
            })
        }
    }
}

fn client_request<'data>(
    connection: &SpeechConnection,
    wiring: &ClientWiring<'_, 'data>,
) -> GetClientRequest<'data> {
    GetClientRequest {
        api_key: connection.api_key.clone(),
        api_base: connection.api_base.clone(),
        api_version: connection.api_version.clone(),
        model: Some(connection.model.clone()),
        is_async: connection.aspeech,
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

// The SDK sends an ad token or a token provider first, then the key.
fn azure_key(
    ad_token: Option<SecretString>,
    provider: Option<TokenProvider<'_>>,
    api_key: Option<SecretString>,
) -> Key<'_> {
    match (ad_token, provider, api_key) {
        (Some(token), _, _) => Key::Bearer(token),
        (None, Some(callable), _) => Key::Provider(callable),
        (None, None, Some(key)) => Key::ApiKey(key),
        (None, None, None) => Key::Absent,
    }
}

// The v1 client keeps the first present of the key, the provider and the ad token.
fn v1_key(key: Option<V1Key<'_>>) -> Key<'_> {
    match key {
        Some(V1Key::Static(static_key)) => Key::Bearer(static_key),
        Some(V1Key::Provider { provider, .. }) => Key::Provider(provider),
        None => Key::Absent,
    }
}

fn azure_fallback<'data>(
    connection: &SpeechConnection,
    custom_query: Option<&BTreeMap<String, String>>,
    wiring: &ClientWiring<'_, 'data>,
) -> Result<Selected<'data>, AzureSpeechError> {
    let selection = select_base_url_or_endpoint(EndpointSelection {
        azure_endpoint: connection.api_base.clone(),
        base_url: None,
    });
    let version = custom_query
        .and_then(|query| query.get("api-version").cloned())
        .or_else(|| connection.api_version.clone())
        .or_else(|| wiring.environment.lookup("AZURE_API_VERSION"))
        .unwrap_or_else(|| wiring.settings.azure_default_api_version.clone());
    let base_url = normalized_base_url(&selection)?;
    Ok(Selected {
        family: Family::Azure,
        base_url,
        api_version: Some(version),
        auth: fallback_key(connection),
    })
}

fn v1_fallback<'data>(connection: &SpeechConnection) -> Result<Selected<'data>, AzureSpeechError> {
    let Some(api_base) = connection.api_base.as_deref() else {
        return Err(AzureSpeechError::MissingBase);
    };
    let key = connection.api_key.clone().map(V1Key::Static).or_else(|| {
        connection
            .provider
            .as_ref()
            .map(|provider| V1Key::Provider {
                provider: TokenProvider(Arc::clone(&provider.provider)),
                offload: connection.aspeech,
            })
    });
    let auth = v1_key(key.or_else(|| connection.params.azure_ad_token.clone().map(V1Key::Static)));
    Ok(Selected {
        family: Family::OpenAiV1,
        base_url: with_trailing_slash(&format!("{api_base}/openai/v1/"))?,
        api_version: None,
        auth,
    })
}

fn fallback_key<'data>(connection: &SpeechConnection) -> Key<'data> {
    let provider: Option<TokenProvider<'data>> = connection
        .provider
        .as_ref()
        .map(|callable| TokenProvider(Arc::clone(&callable.provider)));
    azure_key(
        connection.params.azure_ad_token.clone(),
        provider,
        connection.api_key.clone(),
    )
}

fn normalized_base_url(selection: &EndpointSelection) -> Result<String, AzureSpeechError> {
    let raw = match (&selection.base_url, &selection.azure_endpoint) {
        (Some(base), _) => base.clone(),
        (None, Some(endpoint)) => format!("{}/openai", endpoint.trim_end_matches('/')),
        (None, None) => return Err(AzureSpeechError::MissingBase),
    };
    with_trailing_slash(&raw)
}

fn with_trailing_slash(raw: &str) -> Result<String, AzureSpeechError> {
    let mut parsed = HttpxUrl::parse(raw).map_err(AzureSpeechError::BaseUrl)?;
    if !parsed.path.ends_with('/') {
        parsed.path.push('/');
    }
    Ok(parsed.href())
}

pub fn request_auth(
    selected: &Selected<'_>,
    caller_auth: bool,
) -> Result<Option<RequestAuth>, AzureSpeechError> {
    match &selected.auth {
        Key::Bearer(token) => Ok(Some(RequestAuth::Bearer(token.clone()))),
        Key::Provider(provider) => Ok(Some(RequestAuth::Bearer(provider_token(provider)?))),
        Key::ApiKey(key) => {
            if key.expose_secret().is_empty() {
                return no_credential(selected.family, caller_auth);
            }
            Ok(Some(RequestAuth::ApiKey(key.clone())))
        }
        Key::Absent => no_credential(selected.family, caller_auth),
    }
}

const fn no_credential(
    family: Family,
    caller_auth: bool,
) -> Result<Option<RequestAuth>, AzureSpeechError> {
    match family {
        Family::OpenAiV1 => Err(AzureSpeechError::MissingKey),
        Family::Azure if caller_auth => Ok(None),
        Family::Azure => Err(AzureSpeechError::UnableToHandleAuth),
    }
}

fn provider_token(provider: &TokenProvider<'_>) -> Result<SecretString, AzureSpeechError> {
    let token = provider
        .0
        .token()
        .map_err(|failure| AzureSpeechError::Handler {
            class: failure.class,
            status: None,
            message: failure.message,
        })?;
    let returned = match token {
        TokenReply::Text(text) if !text.expose_secret().is_empty() => return Ok(text),
        TokenReply::Text(_) => String::new(),
        TokenReply::NonText { type_name } => type_name,
    };
    // the SDK names the reply it could not use; an empty text reply prints nothing, since it can hold a credential
    Err(AzureSpeechError::Handler {
        class: String::from("ValueError"),
        status: None,
        message: format!(
            "Expected `azure_ad_token_provider` argument to return a string but it returned {returned}"
        ),
    })
}
