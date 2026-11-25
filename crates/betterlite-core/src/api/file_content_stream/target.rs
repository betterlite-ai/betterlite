use std::collections::BTreeMap;
use std::error::Error;
use std::future::Future;
use std::num::NonZeroUsize;
use std::sync::Arc;

use betterlite_azure::endpoints::projection::LiteLlmParams;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::controls::call_params::TrustedCredentials;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::files::records::{BinaryContent, FileContentRequest};
use betterlite_openai::files::stream::FileContentStreamingResult;
use chrono::{DateTime, FixedOffset};
use secrecy::SecretString;

use crate::api::file_http_content::config::ContentParams;
use crate::api::file_stream_logging::logging::FileStreamLogger;
use crate::api::resource_sdk_settings::timeout::FineTuningTimeout;
use crate::api::vector_store_http_write::service::SuppliedClient;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContentMode {
    pub is_async: bool,
    pub timeout: FineTuningTimeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuppliedSdk {
    OpenAi,
    Azure,
}

pub enum CallerClient<'call, Client> {
    // an OpenAI or Azure SDK client, whose transport never reaches this crate
    Sdk(SuppliedSdk),
    // HTTPHandler or AsyncHTTPHandler
    Http(SuppliedClient<'call, Client>),
}

#[derive(Debug, Clone)]
pub struct CompatibleContent {
    pub api_base: String,
    pub api_key: Option<SecretString>,
    pub organization: Option<String>,
    pub max_retries: Option<i64>,
    pub mode: ContentMode,
    // only file_content_streaming forwards a client, and only an OpenAI or AsyncOpenAI one
    pub client: Option<SuppliedSdk>,
}

// AzureOpenAIFilesAPI's arguments after get_azure_credentials
#[derive(Debug, Clone)]
pub struct AzureContent {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_version: Option<String>,
    pub max_retries: Option<i64>,
    pub params: LiteLlmParams,
    pub mode: ContentMode,
    pub client: Option<SuppliedSdk>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewLogger {
    pub call_type: &'static str,
    pub start_time: DateTime<FixedOffset>,
    pub litellm_call_id: String,
    pub function_id: String,
}

pub enum ContentLogger<'call> {
    Supplied(&'call mut LoggingState),
    Created(Box<NewLogger>),
}

pub struct GenericContent<'call, Config, Client> {
    pub config: Config,
    pub params: ContentParams,
    pub trusted: Option<&'call TrustedCredentials>,
    pub headers: BTreeMap<String, String>,
    pub logger: ContentLogger<'call>,
    pub client: Option<SuppliedClient<'call, Client>>,
    pub mode: ContentMode,
}

pub struct XaiBatchContent<'call> {
    pub batch_id: &'call str,
    pub api_base: Option<&'call str>,
    pub api_key: Option<&'call SecretString>,
    pub timeout: FineTuningTimeout,
}

pub enum ContentTarget<'call, Config, Client> {
    Compatible(CompatibleContent),
    Azure(Box<AzureContent>),
    Generic(Box<GenericContent<'call, Config, Client>>),
    XaiBatch(XaiBatchContent<'call>),
}

pub enum ContentOperation<'call> {
    Buffered {
        request: &'call FileContentRequest,
    },
    Streamed {
        request: &'call FileContentRequest,
        chunk_size: Option<NonZeroUsize>,
    },
}

pub enum ContentReply {
    Buffered(Box<BinaryContent>),
    Streamed(Box<FileContentStreamingResult>),
}

// The values file_content_streaming writes onto the logging object before its provider branch
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamIdentity<'call> {
    // model or "", written to the logging object's model and to model_call_details["model"]
    pub model: &'call str,
    // model_call_details["custom_llm_provider"], always named on a streamed branch
    pub custom_llm_provider: &'call str,
    // model_call_details["litellm_params"]["api_base"], written only when the caller's value is not null
    pub api_base: Option<&'call str>,
}

pub trait ContentLogging: Send + Sync {
    fn record_identity(&self, identity: &StreamIdentity<'_>);

    fn stream_logger(self: Arc<Self>) -> Arc<dyn FileStreamLogger>;
}

pub trait FileContentDispatch: Send + Sync {
    type Config: Send;
    type Client: Sync;
    type Logging: ContentLogging;
    type Error: Error + Send + Sync + 'static;

    // B2464's get_provider_files_config, which lives in the provider crate above this one
    fn files_config(&self, provider: ProviderId) -> Option<Self::Config>;

    // B247's Logging constructor over the seed _file_content_logging_obj built
    fn create_logging(&self, seed: &NewLogger) -> Arc<Self::Logging>;

    fn content(
        &self,
        target: ContentTarget<'_, Self::Config, Self::Client>,
        operation: ContentOperation<'_>,
    ) -> impl Future<Output = Result<ContentReply, Self::Error>> + Send;
}
