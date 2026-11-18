pub mod config;
pub mod error;
mod prepare;
mod send;
mod tests;
pub mod transport;

use std::time::Duration;

use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use futures_util::future::LocalBoxFuture;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::anthropic_http_stream::call::{StreamBuild, StreamLogger};
use crate::anthropic_http_stream::completion::WrappedStream;
use crate::credentials::AnthropicSettings;
use crate::response::wire::ResponseEnvelope;
use crate::stream_state::state::IdSource;
use crate::usage::ReasoningTokenCounter;

use config::ChatConfig;
use error::ChatCallError;
use prepare::Prepared;
use send::streams::{StreamFacts, open_stream};
use transport::{ChatTransport, SuppliedChatClient};
// embedding() is an empty pass body: it builds no request, sends none and returns None
pub fn embedding() -> Option<ResponseEnvelope> {
    None
}
// LiteLLM runs acompletion for an async call and completion for a sync one, and each branch takes its own clients
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallStyle {
    Async,
    Sync,
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
    pub supplied: Option<SuppliedChatClient<'clients, Client>>,
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
pub enum ChatOutcome<ClientFailure, Counter> {
    Reply(Box<ResponseEnvelope>),
    Stream(Box<WrappedStream<ClientFailure, Counter>>),
}
// Every branch answers this outcome or raises this error, carrying the client's and counter's own failures
pub type ChatResult<Client, Counter> = Result<
    ChatOutcome<
        <Client as crate::anthropic_http_stream::transport::StreamTransport>::Failure,
        Counter,
    >,
    ChatCallError<
        <Client as crate::anthropic_http_stream::transport::StreamTransport>::Failure,
        <Counter as ReasoningTokenCounter>::Failure,
    >,
>;
// LiteLLM's AnthropicChatCompletion.completion around the selected config: prepare, filter, log, then send
pub fn chat_call<'call, Config, Client, Counter>(
    call: ChatCall<'call>,
    mut deps: ChatDeps<'call, Config, Client, Counter>,
) -> LocalBoxFuture<'call, ChatResult<Client, Counter>>
where
    Config: ChatConfig,
    Client: ChatTransport,
    Counter: ReasoningTokenCounter + Unpin + Send,
{
    Box::pin(async move {
        let prepared = prepare::prepare(&call, &mut deps).await?;
        if prepared.options.stream {
            return send_stream(call, prepared, deps).await;
        }
        send::buffered(call, prepared, deps).await
    })
}
// the stream branch: acompletion_stream_function takes only an AsyncHTTPHandler, make_sync_call any supplied client
async fn send_stream<Config, Client, Counter>(
    call: ChatCall<'_>,
    prepared: Prepared,
    deps: ChatDeps<'_, Config, Client, Counter>,
) -> ChatResult<Client, Counter>
where
    Client: ChatTransport,
    Counter: ReasoningTokenCounter + Unpin + Send,
{
    // the stream helper takes the messages as one JSON list, the way LiteLLM passes its copied list
    let messages = Json::List(call.messages);
    let facts = StreamFacts {
        model: call.model,
        api_base: call.api_base,
        messages: &messages,
        data: prepared.body,
        headers: prepared.header_map,
        timeout: call.timeout,
        json_mode: prepared.options.json_mode,
        optional_params: &prepared.optional_params,
        litellm_params: &call.litellm_params,
    };
    let build = StreamBuild {
        ids: deps.ids,
        counter: deps.counter,
        max_event_bytes: deps.max_event_bytes,
        max_error_body_bytes: deps.max_error_body_bytes,
    };
    open_stream(
        deps.clients,
        deps.stream_logger,
        build,
        facts,
        call.style == CallStyle::Async,
    )
    .await
}
