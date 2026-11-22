use std::error::Error;
use std::future::Future;

use betterlite_models::controls::call_params::TrustedCredentials;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::files::records::{
    CreatePurpose, ExpiresAfter, FileCreate, FileDeleted, FileObject, FileUpload,
};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::SecretString;
use thiserror::Error;

use crate::api::file_http_upload::service::Mode;
use crate::api::resource_sdk_settings::ResourceSdkRequestOptions;
use crate::api::resource_sdk_settings::timeout::{
    FineTuningTimeout, StructuredTimeout, TimeoutError, TimeoutValue,
};

mod arguments;
pub mod service;
mod tests;

// GenericLiteLLMParams.timeout: float, numeric text or an httpx.Timeout
#[derive(Debug, Clone, PartialEq)]
pub enum WriteTimeout {
    Value(TimeoutValue),
    Structured(StructuredTimeout),
}
// the client kwarg as the branches tell it apart: an httpx handler of one mode, or a provider SDK client
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuppliedClient<Client> {
    Handler { mode: Mode, client: Client },
    Sdk(Client),
}
impl<Client> SuppliedClient<Client> {
    // the generic handler takes a supplied client only when it is an httpx handler
    fn as_handler(&self) -> Option<&Self> {
        match self {
            Self::Handler { .. } => Some(self),
            Self::Sdk(_) => None,
        }
    }
}
// the GenericLiteLLMParams fields both calls read, and the kwargs they read beside it
#[derive(Debug, Clone)]
pub struct WriteParams<Logger, Client> {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_version: Option<String>,
    pub organization: Option<String>,
    pub timeout: Option<WriteTimeout>,
    pub request_timeout: Option<TimeoutValue>,
    pub max_retries: Option<i64>,
    pub passthrough: Option<bool>,
    // absent draws a UUID for delete's fallback logger, and an explicit null is kept as LiteLLM keeps it
    pub litellm_call_id: Option<Option<String>>,
    pub id: Option<String>,
    pub logger: Option<Logger>,
    pub client: Option<SuppliedClient<Client>>,
    pub credentials: Option<TrustedCredentials>,
}
impl<Logger, Client> From<&WriteParams<Logger, Client>> for ResourceSdkRequestOptions {
    fn from(params: &WriteParams<Logger, Client>) -> Self {
        Self {
            api_base: params.api_base.clone(),
            api_version: params.api_version.clone(),
            api_key: params.api_key.clone(),
            organization: params.organization.clone(),
            vertex_project: None,
            vertex_location: None,
            vertex_credentials: None,
        }
    }
}
#[derive(Debug, Clone)]
pub struct UploadInput<Logger, Client> {
    pub file: FileUpload,
    pub purpose: CreatePurpose,
    pub expires_after: Option<ExpiresAfter>,
    // absent or null takes the function's default: None for create_file, openai for acreate_file
    pub custom_llm_provider: Option<Option<String>>,
    pub extra_headers: Option<IndexMap<String, String>>,
    pub extra_body: Option<IndexMap<String, String>>,
    pub params: WriteParams<Logger, Client>,
}
#[derive(Debug, Clone)]
pub struct DeleteInput<Logger, Client> {
    pub file_id: String,
    pub model: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub extra_headers: Option<IndexMap<String, String>>,
    // file_delete accepts extra_body and no branch reads it
    pub extra_body: Option<IndexMap<String, String>>,
    pub params: WriteParams<Logger, Client>,
}
// what get_openai_credentials or get_azure_credentials resolved, with the call's timeout and retries
#[derive(Debug, Clone)]
pub struct ProviderCall {
    pub mode: Mode,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_version: Option<String>,
    pub organization: Option<String>,
    pub timeout: FineTuningTimeout,
    pub max_retries: Option<i64>,
}
// base_llm_http_handler.create_file's arguments; litellm_params is the caller's kwargs, unfiltered
pub struct ConfigUpload<'call, P: FileWriteProviders + ?Sized> {
    pub config: P::Config,
    pub mode: Mode,
    pub data: FileCreate,
    pub headers: IndexMap<String, String>,
    pub api_base: Option<&'call str>,
    pub api_key: Option<&'call SecretString>,
    pub params: &'call WriteParams<P::Logger, P::Client>,
    pub logger: &'call P::Logger,
    pub client: Option<&'call SuppliedClient<P::Client>>,
    pub timeout: FineTuningTimeout,
}
pub enum UploadTarget<'call, P: FileWriteProviders + ?Sized> {
    Config(ConfigUpload<'call, P>),
    OpenAi {
        call: ProviderCall,
        data: FileCreate,
    },
    Azure {
        call: ProviderCall,
        data: FileCreate,
        params: &'call WriteParams<P::Logger, P::Client>,
    },
}
// get_litellm_params' api_key and api_base, assigned even when null, beside the trusted snapshot
#[derive(Debug, Clone)]
pub struct DeleteParams<'call> {
    pub api_key: Option<&'call SecretString>,
    pub api_base: Option<&'call str>,
    pub credentials: Option<&'call TrustedCredentials>,
}
// the Logging object file_delete builds when the caller passed none
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoggerSeed {
    pub call_type: &'static str,
    pub start_time: DateTime<FixedOffset>,
    pub litellm_call_id: Option<String>,
    pub function_id: String,
}
pub enum DeleteLogger<'call, Logger> {
    Existing(&'call Logger),
    Created(LoggerSeed),
}
pub struct ConfigDelete<'call, P: FileWriteProviders + ?Sized> {
    pub config: P::Config,
    pub mode: Mode,
    pub file_id: &'call str,
    pub params: DeleteParams<'call>,
    pub headers: IndexMap<String, String>,
    pub logger: DeleteLogger<'call, P::Logger>,
    pub client: Option<&'call SuppliedClient<P::Client>>,
    pub timeout: FineTuningTimeout,
}
pub enum DeleteTarget<'call, P: FileWriteProviders + ?Sized> {
    OpenAi {
        call: ProviderCall,
        file_id: &'call str,
    },
    Azure {
        call: ProviderCall,
        file_id: &'call str,
        client: Option<&'call SuppliedClient<P::Client>>,
        params: DeleteParams<'call>,
    },
    Config(ConfigDelete<'call, P>),
}
// the provider methods both calls end in; B2464's files config lookup comes in here because it lives above core
pub trait FileWriteProviders: Sync {
    type Config: Copy + Send + Sync;
    type Logger: Sync;
    type Client: Sync;
    type Error: Error + Send + Sync + 'static;

    fn files_config(&self, provider: ProviderId) -> Option<Self::Config>;

    fn upload(
        &self,
        target: UploadTarget<'_, Self>,
    ) -> impl Future<Output = Result<FileObject, Self::Error>> + Send;

    fn delete(
        &self,
        target: DeleteTarget<'_, Self>,
    ) -> impl Future<Output = Result<FileDeleted, Self::Error>> + Send;
}
// the synthesized httpx.Response a LiteLLM BadRequestError carries for a rejected write
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadRequest<'error> {
    pub model: &'static str,
    pub llm_provider: &'error str,
    pub body: &'static str,
    pub method: &'static str,
    pub url: &'static str,
}
#[derive(Debug, Error)]
pub enum FileWriteError<ProviderError: Error + 'static> {
    #[error("logging_obj is required")]
    LoggingObjectRequired,
    #[error(
        "`passthrough=True` uploads the file bytes unchanged for a native Vertex AI batch, so it needs custom_llm_provider='vertex_ai' and purpose='batch', got '{}' and '{purpose}'.",
        .provider.as_deref().unwrap_or("None")
    )]
    Passthrough {
        provider: Option<String>,
        purpose: &'static str,
    },
    #[error("could not convert string to float: '{text}'")]
    TimeoutText {
        text: String,
        #[source]
        source: TimeoutError,
    },
    #[error(transparent)]
    Timeout(TimeoutError),
    #[error("{provider} is not a valid LlmProviders")]
    UnknownProvider { provider: String },
    #[error(
        "LiteLLM doesn't support {provider} for 'create_file'. Only ['openai', 'azure', 'vertex_ai', 'manus', 'anthropic'] are supported."
    )]
    UnsupportedUpload { provider: String },
    #[error(
        "LiteLLM doesn't support {provider} for 'file_delete'. Only 'openai', 'azure', 'gemini', 'manus', and 'anthropic' are supported."
    )]
    UnsupportedDelete { provider: String },
    #[error(transparent)]
    Provider(ProviderError),
}
impl<ProviderError: Error + 'static> FileWriteError<ProviderError> {
    // the three BadRequestErrors name no model; upload's guard reports a missing provider as n/a, and httpx upper-cases the method
    #[must_use]
    pub fn bad_request(&self) -> Option<BadRequest<'_>> {
        let (llm_provider, body, method) = match self {
            Self::Passthrough { provider, .. } => (
                provider.as_deref().unwrap_or("n/a"),
                "passthrough needs a vertex_ai batch",
                "CREATE_FILE",
            ),
            Self::UnsupportedUpload { provider } => {
                (provider.as_str(), "Unsupported provider", "CREATE_FILE")
            }
            Self::UnsupportedDelete { provider } => {
                (provider.as_str(), "Unsupported provider", "CREATE_THREAD")
            }
            Self::LoggingObjectRequired
            | Self::TimeoutText { .. }
            | Self::Timeout(_)
            | Self::UnknownProvider { .. }
            | Self::Provider(_) => return None,
        };
        Some(BadRequest {
            model: "n/a",
            llm_provider,
            body,
            method,
            url: "https://github.com/BerriAI/litellm",
        })
    }
}
