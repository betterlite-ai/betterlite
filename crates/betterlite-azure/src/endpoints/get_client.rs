use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::azure_endpoints::Settings;
use secrecy::SecretString;

use crate::azure_urls::V1_API_VERSIONS;
use crate::credentials::cache::RefreshProviderCache;
use crate::credentials::source::CredentialSources;

use super::identity::client_identity;
use super::initialize::{EndpointError, InitializeRequest, initialize_azure_sdk_client};
use super::projection::{
    AzureClientInputs, CallableProvider, ClientIdentity, ClientInputs, ClientKind, ClientOutcome,
    ClientSource, LiteLlmParams, OpenAiV1ClientInputs, SessionKind, SuppliedClient, V1Key,
};

// the shared cached-client accessors: the identity key, and whether this caller may close the stored client
pub trait ClientCache {
    fn get(&self, identity: &ClientIdentity) -> Option<ClientKind>;
    fn set(&mut self, identity: ClientIdentity, kind: ClientKind, owned: bool);
}

pub struct GetClientRequest<'data> {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub model: Option<String>,
    pub is_async: bool,
    pub params: LiteLlmParams,
    pub provider: Option<CallableProvider>,
    pub supplied: Option<SuppliedClient>,
    pub settings: &'data Settings,
    pub environment: &'data dyn EnvironmentLookup,
    pub refresh_enabled: bool,
    pub session: SessionKind,
    pub sources: &'data dyn CredentialSources,
}

pub fn get_azure_openai_client<'data>(
    request: &GetClientRequest<'data>,
    cache: &mut dyn ClientCache,
    refresh_cache: &mut RefreshProviderCache<'data>,
) -> Result<ClientOutcome<'data>, EndpointError> {
    let identity = client_identity(
        request.api_key.as_ref(),
        request.is_async,
        &request.params,
        request.provider.as_ref(),
    );
    if let Some(supplied) = request.supplied.as_ref() {
        return Ok(supplied_outcome(
            supplied,
            request.api_version.as_deref(),
            identity,
            cache,
        ));
    }
    if let Some(outcome) = cached_outcome(&identity, cache) {
        return Ok(outcome);
    }
    construct(request, cache, refresh_cache, identity)
}

fn supplied_outcome<'data>(
    supplied: &SuppliedClient,
    api_version: Option<&str>,
    identity: ClientIdentity,
    cache: &mut dyn ClientCache,
) -> ClientOutcome<'data> {
    let mut custom_query = supplied.custom_query.clone();
    if let Some(version) = api_version
        && matches!(
            supplied.kind,
            ClientKind::AzureOpenAi | ClientKind::AsyncAzureOpenAi
        )
        && let Some(query) = custom_query.as_mut()
    {
        query
            .entry("api-version".to_owned())
            .or_insert_with(|| version.to_owned());
    }
    cache.set(identity.clone(), supplied.kind, false);
    ClientOutcome {
        source: ClientSource::Supplied,
        kind: supplied.kind,
        inputs: None,
        custom_query,
        cache_owned: Some(false),
        identity,
    }
}

// any cached SDK client of one of the four families is returned as it is; every other value is ignored
fn cached_outcome<'data>(
    identity: &ClientIdentity,
    cache: &dyn ClientCache,
) -> Option<ClientOutcome<'data>> {
    let kind = cache.get(identity)?;
    if !matches!(
        kind,
        ClientKind::AzureOpenAi
            | ClientKind::AsyncAzureOpenAi
            | ClientKind::OpenAi
            | ClientKind::AsyncOpenAi
    ) {
        return None;
    }
    Some(ClientOutcome {
        source: ClientSource::Cached,
        kind,
        inputs: None,
        custom_query: None,
        cache_owned: None,
        identity: identity.clone(),
    })
}

fn construct<'data>(
    request: &GetClientRequest<'data>,
    cache: &mut dyn ClientCache,
    refresh_cache: &mut RefreshProviderCache<'data>,
    identity: ClientIdentity,
) -> Result<ClientOutcome<'data>, EndpointError> {
    let azure = initialize_azure_sdk_client(&initialize_request(request), refresh_cache)?;
    // the original api_version argument picks the client family, never the value resolved from the environment
    let v1 = request
        .api_version
        .as_deref()
        .is_some_and(|version| V1_API_VERSIONS.contains(&version));
    let kind = match (v1, request.is_async) {
        (true, true) => ClientKind::AsyncOpenAi,
        (true, false) => ClientKind::OpenAi,
        (false, true) => ClientKind::AsyncAzureOpenAi,
        (false, false) => ClientKind::AzureOpenAi,
    };
    let owned = !azure.http_client.shared;
    let inputs = if v1 {
        ClientInputs::OpenAiV1(v1_inputs(
            azure,
            request.api_base.as_deref(),
            request.is_async,
        ))
    } else {
        ClientInputs::Azure(azure)
    };
    cache.set(identity.clone(), kind, owned);
    Ok(ClientOutcome {
        source: ClientSource::Constructed,
        kind,
        inputs: Some(inputs),
        custom_query: None,
        cache_owned: Some(owned),
        identity,
    })
}

fn initialize_request<'data>(request: &GetClientRequest<'data>) -> InitializeRequest<'data> {
    InitializeRequest {
        params: request.params.clone(),
        provider: request.provider.clone(),
        api_key: request.api_key.clone(),
        api_base: request.api_base.clone(),
        model_name: request.model.clone(),
        api_version: request.api_version.clone(),
        is_async: request.is_async,
        session: request.session,
        refresh_enabled: request.refresh_enabled,
        settings: request.settings,
        environment: request.environment,
        sources: request.sources,
    }
}

// LiteLLM takes the first present credential as the standard client key, and wraps a provider for async
fn v1_inputs<'data>(
    azure: AzureClientInputs<'data>,
    api_base: Option<&str>,
    is_async: bool,
) -> OpenAiV1ClientInputs<'data> {
    let AzureClientInputs {
        api_key,
        token_provider,
        azure_ad_token,
        timeout,
        max_retries,
        http_client,
        ..
    } = azure;
    let key = api_key.map(V1Key::Static).or_else(|| {
        token_provider.map(|provider| V1Key::Provider {
            provider,
            offload: is_async,
        })
    });
    OpenAiV1ClientInputs {
        key: key.or_else(|| azure_ad_token.map(V1Key::Static)),
        base_url: api_base.map(|base| format!("{base}/openai/v1/")),
        timeout,
        max_retries,
        http_client,
    }
}
