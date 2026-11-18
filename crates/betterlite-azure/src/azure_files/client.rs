pub mod auth;

mod target;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::azure_endpoints::Settings as EndpointSettings;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_openai::files::FilesClient;
use http::header::{AUTHORIZATION, HeaderName};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::credentials::cache::RefreshProviderCache;
use crate::credentials::source::CredentialSources;
use crate::endpoints::get_client::{ClientCache, GetClientRequest, get_azure_openai_client};
use crate::endpoints::projection::{
    AzureClientInputs, CallableProvider, ClientInputs, ClientKind, ClientOutcome, LiteLlmParams,
    OpenAiV1ClientInputs, SessionKind, SuppliedClient, V1Key,
};

use self::auth::{AuthHeader, AuthKind, FileAuth, provider_auth, resolve_auth};
use self::target::{azure_base_url, client_timeout, parse_base, retries};
use super::error::AzureFilesError;

pub(crate) const API_KEY: HeaderName = HeaderName::from_static("api-key");

// The api_base, api_key and api_version arguments of the handler, without the ones it never forwards
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ClientSettings {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub api_version: Option<String>,
}

// What a supplied or cached client's own construction left behind: the SDK client LiteLLM returns as it is
pub trait ClientMaterializer {
    fn materialize(
        &self,
        outcome: &ClientOutcome<'_>,
    ) -> Result<Option<SelectedClient>, AzureFilesError>;
}

pub struct Connection<'cache, 'data, M: ClientMaterializer> {
    pub settings: ClientSettings,
    // litellm_params: the only place the handler forwards timeout, max_retries, the ad token and its provider from
    pub params: LiteLlmParams,
    pub provider: Option<CallableProvider>,
    pub supplied: Option<SuppliedClient>,
    pub is_async: bool,
    pub endpoints: &'data EndpointSettings,
    pub environment: &'data dyn EnvironmentLookup,
    pub sources: &'data dyn CredentialSources,
    pub session: SessionKind,
    pub refresh_enabled: bool,
    pub cache: &'cache mut dyn ClientCache,
    pub refresh: &'cache mut RefreshProviderCache<'data>,
    pub materializer: &'cache M,
}

impl<'data, M: ClientMaterializer> Connection<'_, 'data, M> {
    pub fn get_openai_client(&mut self) -> Result<ClientOutcome<'data>, AzureFilesError> {
        let request = GetClientRequest {
            api_key: self.settings.api_key.clone(),
            api_base: self.settings.api_base.clone(),
            api_version: self.settings.api_version.clone(),
            model: None,
            is_async: self.is_async,
            params: self.params.clone(),
            provider: self.provider.clone(),
            supplied: self.supplied.clone(),
            settings: self.endpoints,
            environment: self.environment,
            refresh_enabled: self.refresh_enabled,
            session: self.session,
            sources: self.sources,
        };
        get_azure_openai_client(&request, &mut *self.cache, &mut *self.refresh)
            .map_err(AzureFilesError::from)
    }

    // LiteLLM builds the client on every call and keeps a supplied one, so a cached outcome comes back too
    pub fn open(mut self) -> Result<SelectedClient, AzureFilesError> {
        let is_async = self.is_async;
        let outcome = self.get_openai_client()?;
        let kind = outcome.kind;
        let client = match outcome.inputs.as_ref() {
            Some(ClientInputs::Azure(azure)) => SelectedClient::from_azure(azure, kind)?,
            Some(ClientInputs::OpenAiV1(v1)) => SelectedClient::from_v1(v1, kind)?,
            None => self
                .materializer
                .materialize(&outcome)?
                .ok_or(AzureFilesError::ClientMissing)?,
        };
        // the guard LiteLLM runs after its None check; the sync path has none, so an async client passes there
        if is_async && !client.is_async() {
            return Err(AzureFilesError::NotAsyncClient);
        }
        Ok(client)
    }
}

// The request builder every file operation sends through, plus the query and auth the Azure clients add
pub struct SelectedClient {
    pub files: FilesClient,
    pub api_version: Option<String>,
    pub kind: ClientKind,
    auth: Option<AuthHeader>,
    base_query: Vec<(String, String)>,
}

impl SelectedClient {
    pub fn new(
        files: FilesClient,
        auth: FileAuth,
        api_version: Option<String>,
        kind: ClientKind,
    ) -> Result<Self, AzureFilesError> {
        let base_query: Vec<(String, String)> = files.base_url.query_pairs().into_owned().collect();
        Ok(Self {
            files,
            api_version,
            kind,
            auth: auth.header()?,
            base_query,
        })
    }

    #[must_use]
    pub const fn is_async(&self) -> bool {
        matches!(
            self.kind,
            ClientKind::AsyncAzureOpenAi | ClientKind::AsyncOpenAi
        )
    }

    fn from_azure(
        azure: &AzureClientInputs<'_>,
        kind: ClientKind,
    ) -> Result<Self, AzureFilesError> {
        let auth = resolve_auth(
            azure.azure_ad_token.clone(),
            azure.token_provider.as_ref(),
            azure.api_key.clone(),
        )?;
        let files = FilesClient {
            api_key: auth.secret(),
            base_url: azure_base_url(&azure.endpoint)?,
            organization: None,
            timeout: client_timeout(azure.timeout)?,
            max_retries: retries(azure.max_retries),
        };
        Self::new(files, auth, Some(azure.api_version.clone()), kind)
    }

    fn from_v1(v1: &OpenAiV1ClientInputs<'_>, kind: ClientKind) -> Result<Self, AzureFilesError> {
        // B531 picked the v1 key in LiteLLM's order: api_key, then the provider, then the ad token
        let auth = match v1.key.as_ref() {
            Some(V1Key::Static(secret)) if !secret.expose_secret().is_empty() => {
                FileAuth::Bearer(secret.clone())
            }
            Some(V1Key::Static(_)) | None => FileAuth::None,
            Some(V1Key::Provider { provider, .. }) => provider_auth(provider)?,
        };
        let raw = v1
            .base_url
            .as_deref()
            .ok_or(AzureFilesError::MissingBaseUrl)?;
        let files = FilesClient {
            api_key: auth.secret(),
            base_url: parse_base(raw)?,
            organization: None,
            timeout: client_timeout(v1.timeout)?,
            max_retries: retries(v1.max_retries),
        };
        Self::new(files, auth, None, kind)
    }

    // The SDK adds its auth header unless the caller's extra headers carry that name, and puts
    // default_query after any base URL query but before the request's own params
    pub(super) fn sign(
        &self,
        mut request: OutboundRequest,
        extra_headers: Option<&IndexMap<String, String>>,
    ) -> OutboundRequest {
        let supplied = |name: &str| {
            extra_headers
                .is_some_and(|headers| headers.keys().any(|key| key.eq_ignore_ascii_case(name)))
        };
        // the request builder always starts from a bearer header; the Azure client replaces it as needed
        if !supplied("authorization") {
            request.headers.remove(AUTHORIZATION);
        }
        if let Some(header) = self.auth.as_ref() {
            match header.kind {
                AuthKind::ApiKey if !supplied("api-key") => {
                    request
                        .headers
                        .insert(API_KEY.clone(), header.value.clone());
                }
                AuthKind::Bearer if !supplied("authorization") => {
                    request.headers.insert(AUTHORIZATION, header.value.clone());
                }
                AuthKind::ApiKey | AuthKind::Bearer | AuthKind::None => {}
            }
        }
        let mut query = self.query();
        query.extend(request.query);
        request.query = query;
        request
    }

    // The query the SDK client puts on every request: its constructor query, with api-version replaced
    #[must_use]
    pub fn query(&self) -> Vec<(String, String)> {
        let mut query = self.base_query.clone();
        if let Some(version) = &self.api_version {
            match query.iter_mut().find(|(name, _)| name == "api-version") {
                Some((_, value)) => value.clone_from(version),
                None => query.push(("api-version".to_owned(), version.clone())),
            }
        }
        query
    }

    #[must_use]
    pub fn auth_kind(&self) -> AuthKind {
        self.auth
            .as_ref()
            .map_or(AuthKind::None, |header| header.kind)
    }
}
