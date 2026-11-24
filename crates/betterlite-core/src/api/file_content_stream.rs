use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use betterlite_config::sections::resource_sdk_settings::Settings;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::controls::call_params::TrustedCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::files::records::BinaryContent;
use betterlite_openai::files::stream::StreamExit;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::SecretString;
use uuid::Uuid;

use crate::api::file_stream_logging::FileContentStreamingResponse;
use crate::api::fine_tuning_read::{ReadTimeout, ReadTimeoutValue};

use self::target::CallerClient;

pub mod error;
mod plan;
mod service;
mod streaming;
pub mod target;

mod tests;

// file_content or afile_content: afile_content's declared default provider is openai, and it is the
// only entry that sets afile_content=true
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentEntry {
    Sync,
    Async,
}

// The arguments both entry points declare; model and custom_llm_provider are the two the body reads
pub struct ContentCall<'call> {
    pub file_id: &'call str,
    pub model: Option<&'call str>,
    // absent or null takes this entry's default
    pub custom_llm_provider: Option<String>,
    pub extra_headers: Option<BTreeMap<String, String>>,
    pub extra_body: Option<BTreeMap<String, String>>,
    pub chunk_size: u64,
    pub stream: bool,
    pub kwargs: &'call ContentKwargs,
}

// The kwargs GenericLiteLLMParams and get_litellm_params read beside the declared arguments
#[derive(Debug, Clone, Default)]
pub struct ContentKwargs {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub organization: Option<String>,
    pub timeout: Option<ReadTimeout>,
    pub request_timeout: Option<ReadTimeoutValue>,
    pub max_retries: Option<i64>,
    pub litellm_call_id: Option<String>,
    pub id: Option<String>,
}

// What only server code hands over: the trusted credential snapshot, the caller's client, the logging
// state the generic handler logs through, and the logging object _file_content_logging_obj returns
pub struct ContentContext<'call, Client, Logging> {
    pub trusted: Option<&'call TrustedCredentials>,
    pub client: Option<CallerClient<'call, Client>>,
    pub logger: Option<&'call mut LoggingState>,
    pub logging: Option<Arc<Logging>>,
}

pub struct FileContentStream<'deps, Dispatch> {
    pub dispatch: Dispatch,
    pub settings: Settings,
    pub resolution: &'deps ResolutionContext<'deps>,
    pub clock: fn() -> DateTime<FixedOffset>,
    pub call_ids: fn() -> Uuid,
}

// The streamed branch's answer: the B1080 wrapper over the SDK stream, the provider's headers and the
// exit the stream context was left in
pub struct StreamedContent {
    pub response: FileContentStreamingResponse,
    pub headers: IndexMap<String, String>,
    pub exit: Arc<OnceLock<StreamExit>>,
}

// HttpxBinaryResponseContent's status, headers and bytes, or the wrapped stream
pub enum ContentAnswer {
    Buffered(Box<BinaryContent>),
    Streamed(Box<StreamedContent>),
}
