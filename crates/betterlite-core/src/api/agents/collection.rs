pub mod creation;
mod errors;
pub mod listing;
mod preparation;

use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use betterlite_error::error::ErrorClass;
use betterlite_http::interactions::agents_create_read::AgentRequestError;
use betterlite_http::interactions::agents_create_read::logging::AgentLogging;
use betterlite_http::interactions::base::clients::HttpClients;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use thiserror::Error;

use super::AgentsApiError;

// LiteLLM's async entry points raise these flags in kwargs, and the sync entry points pop them before HTTP delegation
const CREATE_ASYNC_FLAG: &str = "acreate_agent";
const LIST_ASYNC_FLAG: &str = "alist_agents";
const CREATE_CALL_TYPE: &str = "create_agent";
const LIST_CALL_TYPE: &str = "list_agents";
const PROVIDER_FIELD: &str = "custom_llm_provider";
const DEFAULT_PROVIDER: &str = "gemini";

// LiteLLM's exception_type reads the message and status off a provider error before it picks a class
pub trait AgentFailure: StdError + Send + Sync + 'static {
    fn status_code(&self) -> Option<u16>;

    fn message(&self) -> String;
}

// what a collection call needs besides the call: the config, clients, logging state and sink, redactor, and timeout
pub struct AgentCollection<'deps, Config> {
    pub config: &'deps Config,
    pub clients: &'deps mut HttpClients,
    pub logging: &'deps mut LoggingState,
    pub records: &'deps mut dyn AgentLogging,
    pub redactor: &'deps Redactor,
    pub request_timeout: Duration,
}

// create's arguments after its kwargs, with base_environment kept as the caller sent it (an env id or an object)
pub struct AgentCreation<'call> {
    pub name: &'call str,
    pub base_agent: Option<&'call str>,
    pub instructions: Option<&'call str>,
    pub base_environment: Option<&'call Json>,
    pub custom_llm_provider: Option<&'call str>,
    pub extra_headers: BTreeMap<String, String>,
    pub extra_body: Option<&'call IndexMap<String, Json>>,
    pub timeout: Option<Duration>,
    pub kwargs: &'call IndexMap<String, Json>,
}

// list takes no name and no body: the provider, the caller's headers, a timeout and the kwargs
pub struct AgentListing<'call> {
    pub custom_llm_provider: Option<&'call str>,
    pub extra_headers: BTreeMap<String, String>,
    pub timeout: Option<Duration>,
    pub kwargs: &'call IndexMap<String, Json>,
}

// the sync entry answers with the result, or with the pending async call when the operation's flag asked for one
pub enum AgentCollectionHandled<Output, Pending> {
    Done(Output),
    Pending(Pending),
}

pub type AgentCollectionResult<Output, ProviderError> =
    Result<Output, AgentCollectionFailure<ProviderError>>;

// the call a sync entry point leaves pending: the boxed future the async flag asked it to start; Send because acreate and alist await it
pub type AgentPending<'call, Output, Error> =
    Pin<Box<dyn Future<Output = AgentCollectionResult<Output, Error>> + Send + 'call>>;

// a sync entry point's answer: the result, or the pending call the async flag asked for
pub type AgentAnswer<'call, Output, Error> = Result<
    AgentCollectionHandled<Output, AgentPending<'call, Output, Error>>,
    AgentCollectionFailure<Error>,
>;

#[derive(Debug, Error)]
pub enum AgentPreparationError {
    #[error(transparent)]
    Config(#[from] AgentsApiError),
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    #[error("the {field} value must be a string")]
    FieldType { field: &'static str },
}

#[derive(Debug, Error)]
pub enum AgentCollectionError<ProviderError>
where
    ProviderError: StdError + 'static,
{
    #[error(transparent)]
    Prepare(#[from] AgentPreparationError),
    #[error(transparent)]
    Request(#[from] AgentRequestError<ProviderError>),
}

#[derive(Debug, Error)]
#[error("{error}")]
pub struct AgentCollectionFailure<ProviderError>
where
    ProviderError: StdError + 'static,
{
    #[source]
    pub error: AgentCollectionError<ProviderError>,
    pub class: ErrorClass,
}

include!("collection/tests/modules.rs");
