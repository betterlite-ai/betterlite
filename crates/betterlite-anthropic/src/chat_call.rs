pub mod config;
pub mod error;
pub mod prepare;
pub mod send;
pub mod steps;
mod tests;
pub mod transport;

use std::time::Duration;

use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::anthropic_http_stream::call::StreamLogger;
use crate::anthropic_http_stream::completion::WrappedStream;
use crate::anthropic_http_stream::transport::StreamTransport;
use crate::credentials::AnthropicSettings;
use crate::response::wire::ResponseEnvelope;
use crate::stream_state::state::IdSource;
use crate::usage::ReasoningTokenCounter;

use betterlite_http::client_kind::SuppliedClient;
use config::ChatConfig;
use error::ChatCallError;
use steps::finish;
use transport::ChatTransport;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallStyle {
    Async,
    Sync,
    Embedding,
}
pub struct ChatCall<'call> {
    pub model: &'call str,
    pub custom_llm_provider: &'call str,
    pub messages: Vec<Json>,
    pub optional_params: IndexMap<String, Json>,
    pub litellm_params: IndexMap<String, Json>,
    pub caller_headers: IndexMap<String, String>,
    pub api_base: &'call str,
    pub api_key: Option<&'call SecretString>,
    pub settings: &'call AnthropicSettings,
    pub timeout: Duration,
    pub style: CallStyle,
    pub effort_used: bool,
    pub now: i64,
    pub disable_add_prefix_to_prompt: bool,
    pub model_response: ResponseEnvelope,
}
pub struct ChatClients<'clients, Client> {
    pub supplied: Option<SuppliedClient<'clients, Client>>,
    pub pooled: &'clients Client,
}
pub struct ChatDeps<'deps, Config, Client, Counter> {
    pub config: &'deps Config,
    pub clients: ChatClients<'deps, Client>,
    pub state: &'deps mut LoggingState,
    pub env: &'deps PrePostEnv<'deps>,
    pub stream_logger: &'deps dyn StreamLogger,
    pub counter: Counter,
    pub ids: &'deps dyn IdSource,
    pub max_event_bytes: usize,
    pub max_error_body_bytes: usize,
}
// The request facts every step after prepare reads; the provider's own prepare consumes the rest of its request
pub struct CallFacts<'call> {
    pub model: &'call str,
    pub messages: Vec<Json>,
    pub litellm_params: IndexMap<String, Json>,
    pub api_base: &'call str,
    pub timeout: Duration,
    pub style: CallStyle,
    pub now: i64,
    pub disable_add_prefix_to_prompt: bool,
    pub model_response: ResponseEnvelope,
    pub sync_stream: SyncStream,
}
// The sync wrapper's provider, and whether make_sync_call receives the caller's speed and reverse map.
pub struct SyncStream {
    pub custom_llm_provider: &'static str,
    pub names: SyncStreamNames,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncStreamNames {
    FromParams,
    Omitted,
}
pub enum ChatOutcome<ClientFailure, Counter> {
    Reply(Box<ResponseEnvelope>),
    Stream(Box<WrappedStream<ClientFailure, Counter>>),
    Embedding,
}
pub type ChatResult<Client, Counter> = Result<
    ChatOutcome<<Client as StreamTransport>::Failure, Counter>,
    ChatCallError<
        <Client as StreamTransport>::Failure,
        <Counter as ReasoningTokenCounter>::Failure,
    >,
>;
pub async fn chat_call<'call, Config, Client, Counter>(
    call: ChatCall<'call>,
    mut deps: ChatDeps<'call, Config, Client, Counter>,
) -> ChatResult<Client, Counter>
where
    Config: ChatConfig,
    Client: ChatTransport,
    Counter: ReasoningTokenCounter + Unpin + Send + 'call,
{
    if call.style == CallStyle::Embedding {
        return Ok(ChatOutcome::Embedding);
    }
    let prepared = prepare::prepare(&call, &mut deps).await?;
    finish(CallFacts::from(call), prepared, deps).await
}

impl<'call> From<ChatCall<'call>> for CallFacts<'call> {
    fn from(call: ChatCall<'call>) -> Self {
        Self {
            model: call.model,
            messages: call.messages,
            litellm_params: call.litellm_params,
            api_base: call.api_base,
            timeout: call.timeout,
            style: call.style,
            now: call.now,
            disable_add_prefix_to_prompt: call.disable_add_prefix_to_prompt,
            model_response: call.model_response,
            sync_stream: SyncStream {
                custom_llm_provider: "anthropic",
                names: SyncStreamNames::FromParams,
            },
        }
    }
}
