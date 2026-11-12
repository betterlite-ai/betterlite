use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use secrecy::SecretString;
use serde::Deserialize;

use crate::azure_urls::{AuthParams, EndpointSelection};
use crate::credentials::source::CallableTokenProvider;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    Dedicated,
    Shared,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpClient {
    pub is_async: bool,
    pub shared: bool,
}

// the litellm_params keys the owned Azure client functions read
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiteLlmParams {
    #[serde(default)]
    pub azure_ad_token: Option<SecretString>,
    #[serde(default)]
    pub auth: AuthParams,
    #[serde(default)]
    pub max_retries: Option<i64>,
    #[serde(default)]
    pub timeout: Option<f64>,
}

// Rust has no object id() for a trait object, so the caller assigns the identity LiteLLM reads from id()
#[derive(Clone)]
pub struct CallableProvider {
    pub provider: Arc<dyn CallableTokenProvider>,
    pub identity: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClientIdentity {
    pub is_async: bool,
    pub hashed_api_key: Option<String>,
    pub hashed_ad_token: Option<String>,
    pub provider_identity: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientKind {
    AzureOpenAi,
    AsyncAzureOpenAi,
    OpenAi,
    AsyncOpenAi,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientSource {
    Supplied,
    Cached,
    Constructed,
}

pub struct AzureClientInputs<'data> {
    pub endpoint: EndpointSelection,
    pub api_version: String,
    pub api_key: Option<SecretString>,
    pub azure_ad_token: Option<SecretString>,
    pub token_provider: Option<Arc<dyn CallableTokenProvider + 'data>>,
    pub max_retries: i64,
    pub timeout: Option<f64>,
    pub http_client: HttpClient,
}

impl fmt::Debug for AzureClientInputs<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AzureClientInputs")
            .field("endpoint", &self.endpoint)
            .field("api_version", &self.api_version)
            .field("api_key", &self.api_key)
            .field("azure_ad_token", &self.azure_ad_token)
            .field("token_provider", &self.token_provider.is_some())
            .field("max_retries", &self.max_retries)
            .field("timeout", &self.timeout)
            .field("http_client", &self.http_client)
            .finish()
    }
}

pub enum V1Key<'data> {
    Static(SecretString),
    Provider {
        provider: Arc<dyn CallableTokenProvider + 'data>,
        offload: bool,
    },
}

impl fmt::Debug for V1Key<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Static(_) => f.write_str("Static([REDACTED])"),
            Self::Provider { offload, .. } => f
                .debug_struct("Provider")
                .field("offload", offload)
                .finish(),
        }
    }
}

// LiteLLM only forwards the three optional constructor arguments when the initializer produced them
pub struct OpenAiV1ClientInputs<'data> {
    pub key: Option<V1Key<'data>>,
    pub base_url: Option<String>,
    pub timeout: Option<f64>,
    pub max_retries: i64,
    pub http_client: HttpClient,
}

impl fmt::Debug for OpenAiV1ClientInputs<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiV1ClientInputs")
            .field("key", &self.key)
            .field("base_url", &self.base_url)
            .field("timeout", &self.timeout)
            .field("max_retries", &self.max_retries)
            .field("http_client", &self.http_client)
            .finish()
    }
}

pub enum ClientInputs<'data> {
    Azure(AzureClientInputs<'data>),
    OpenAiV1(OpenAiV1ClientInputs<'data>),
}

impl fmt::Debug for ClientInputs<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Azure(inputs) => fmt::Debug::fmt(inputs, f),
            Self::OpenAiV1(inputs) => fmt::Debug::fmt(inputs, f),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SuppliedClient {
    pub kind: ClientKind,
    pub custom_query: Option<BTreeMap<String, String>>,
}

#[derive(Debug)]
pub struct ClientOutcome<'data> {
    pub source: ClientSource,
    pub kind: ClientKind,
    pub inputs: Option<ClientInputs<'data>>,
    pub custom_query: Option<BTreeMap<String, String>>,
    pub cache_owned: Option<bool>,
    pub identity: ClientIdentity,
}
