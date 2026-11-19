use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use betterlite_logging::logging_pre_post::args::AdditionalArgs;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::text_completion_transport::logging::{SecretHeaders, StreamDefaults};
use betterlite_openai::text_completion_transport::sdk::ByteStream;
use betterlite_streaming::stream_chunk_shaping::creation::{StreamInstant, system_instant};
use http::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;
use uuid::Uuid;

use crate::endpoints::projection::{CallableProvider, ClientKind, LiteLlmParams};

use super::error::Raised;

// The arguments azure.py's streaming() and async_streaming() are called with
pub struct StreamCall<'call> {
    pub api_base: &'call str,
    pub api_key: Option<&'call SecretString>,
    pub api_version: &'call str,
    pub data: &'call Extra,
    pub model: &'call str,
    pub timeout: Option<f64>,
    pub max_retries: i64,
    pub azure_ad_token: Option<&'call SecretString>,
    pub azure_ad_token_provider: Option<&'call CallableProvider>,
    pub litellm_params: &'call LiteLlmParams,
    pub supplied: Option<&'call StreamClient>,
}

// get_azure_openai_client(...) as the streaming paths call it
pub struct GetStreamClient<'get> {
    pub api_version: &'get str,
    pub api_base: &'get str,
    pub api_key: Option<&'get SecretString>,
    pub model: &'get str,
    pub supplied: Option<&'get StreamClient>,
    pub litellm_params: &'get LiteLlmParams,
    pub is_async: bool,
}

pub struct StreamAnswer {
    pub headers: HeaderMap,
    pub body: ByteStream,
}

// The client call that opens the stream, from the transport up to the raw headers
pub trait ChatStreamTransport: Send + Sync {
    fn create<'call>(
        &'call self,
        data: &'call Extra,
        timeout: Option<f64>,
    ) -> Pin<Box<dyn Future<Output = Result<StreamAnswer, Raised>> + Send + 'call>>;
}

#[derive(Clone)]
pub struct StreamClient {
    pub kind: ClientKind,
    pub api_key: Option<SecretString>,
    pub transport: Arc<dyn ChatStreamTransport>,
}

// get_azure_openai_client: the supplied client, else the SDK client the arguments build
pub trait StreamSource: Send + Sync {
    fn get(&self, request: GetStreamClient<'_>) -> Result<StreamClient, Raised>;
}

#[derive(Clone, Copy)]
pub struct StreamRuntime {
    pub now: fn() -> StreamInstant,
    pub next_id: fn() -> Uuid,
}

impl Default for StreamRuntime {
    fn default() -> Self {
        Self {
            now: system_instant,
            next_id: Uuid::new_v4,
        }
    }
}

// logging_obj.pre_call(input=..., api_key=..., additional_args=...): the client's own key is logged
pub struct PreCallRecord {
    pub input: Json,
    pub api_key: Option<SecretString>,
    pub additional_args: AdditionalArgs,
}

// The logging object the streaming paths hold; production implements it over betterlite-logging
pub trait StreamHooks {
    fn pre_call(&mut self, record: PreCallRecord) -> Result<(), PrePostError>;

    // logging_obj.model_call_details["response_headers"] = headers
    fn response_headers(&mut self, headers: &IndexMap<String, String>);

    fn stream_defaults(&self) -> StreamDefaults;
}
