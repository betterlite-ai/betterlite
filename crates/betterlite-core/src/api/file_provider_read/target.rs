use std::collections::BTreeMap;
use std::error::Error;
use std::future::Future;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::controls::call_params::TrustedCredentials;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::files::records::{FileObject, FilePage};
use chrono::{DateTime, FixedOffset};
use secrecy::SecretString;

use crate::api::resource_sdk_settings::timeout::FineTuningTimeout;
use crate::api::vector_store_http_write::service::SuppliedClient;

use super::error::ReadFunction;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReadMode {
    pub is_async: bool,
    pub timeout: FineTuningTimeout,
}

// OpenAIFilesAPI's arguments for openai, hosted_vllm and litellm_proxy, after get_openai_credentials
#[derive(Debug, Clone)]
pub struct CompatibleRead {
    pub api_base: String,
    pub api_key: Option<SecretString>,
    pub organization: Option<String>,
    pub max_retries: Option<i64>,
    pub mode: ReadMode,
}

// AzureOpenAIFilesAPI's arguments after get_azure_credentials
#[derive(Debug, Clone)]
pub struct AzureRead {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_version: Option<String>,
    pub max_retries: Option<i64>,
    pub mode: ReadMode,
}

// The logging object LiteLLM builds when the caller passed none, always with model "", messages [] and stream false
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewLogger {
    pub call_type: &'static str,
    pub start_time: DateTime<FixedOffset>,
    pub litellm_call_id: Option<String>,
    pub function_id: String,
}

pub enum ReadLogger<'call> {
    Supplied(&'call mut LoggingState),
    Created(Box<NewLogger>),
}

// base_llm_http_handler's arguments: the selected config, get_litellm_params' key and base, the headers and the HTTP client
pub struct GenericRead<'call, Config, Client> {
    pub config: Config,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub trusted: Option<&'call TrustedCredentials>,
    pub headers: BTreeMap<String, String>,
    pub logger: ReadLogger<'call>,
    pub client: Option<SuppliedClient<'call, Client>>,
    pub mode: ReadMode,
}

pub enum ReadTarget<'call, Config, Client> {
    Compatible(CompatibleRead),
    Azure(AzureRead),
    Generic(Box<GenericRead<'call, Config, Client>>),
}

impl<'call, Config, Client> From<GenericRead<'call, Config, Client>>
    for ReadTarget<'call, Config, Client>
{
    fn from(read: GenericRead<'call, Config, Client>) -> Self {
        Self::Generic(Box::new(read))
    }
}

// The list shape of the branch that answered, which the dispatcher hands back without a wrapper
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileListing {
    Page(Box<FilePage>),
    Files(Vec<FileObject>),
}

// Which of the four entry points runs, with the one argument it forwards
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadOperation<'call> {
    Retrieve { file_id: &'call str },
    List { purpose: Option<&'call str> },
}

impl<'call> ReadOperation<'call> {
    #[must_use]
    pub const fn function(self) -> ReadFunction {
        match self {
            Self::Retrieve { .. } => ReadFunction::FileRetrieve,
            Self::List { .. } => ReadFunction::FileList,
        }
    }
}

// What the chosen branch answered: retrieve's file object or the list shape that branch hands back
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadAnswer {
    File(Box<FileObject>),
    Listing(FileListing),
}

pub trait FileReadDispatch: Send + Sync {
    type Config: Send;
    type Client: Sync;
    type Error: Error + Send + Sync + 'static;

    // B2464's get_provider_files_config, which lives in the provider crate above this one
    fn files_config(&self, provider: ProviderId) -> Option<Self::Config>;

    fn read(
        &self,
        target: ReadTarget<'_, Self::Config, Self::Client>,
        operation: ReadOperation<'_>,
    ) -> impl Future<Output = Result<ReadAnswer, Self::Error>> + Send;
}
