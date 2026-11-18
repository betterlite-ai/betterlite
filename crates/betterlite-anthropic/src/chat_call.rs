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
use futures_util::FutureExt as _;
use futures_util::future::LocalBoxFuture;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::anthropic_http_stream::call::{StreamBuild, StreamLogger};
use crate::anthropic_http_stream::completion::WrappedStream;
use crate::anthropic_http_stream::transport::StreamTransport;
use crate::credentials::AnthropicSettings;
use crate::response::wire::ResponseEnvelope;
use crate::stream_state::state::IdSource;
use crate::usage::ReasoningTokenCounter;

use config::ChatConfig;
use error::ChatCallError;
use prepare::Prepared;
use send::streams::{StreamFacts, open_stream};
use transport::{ChatTransport, SuppliedChatClient};
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
    Embedding,
}
pub type ChatResult<Client, Counter> = Result<
    ChatOutcome<<Client as StreamTransport>::Failure, Counter>,
    ChatCallError<
        <Client as StreamTransport>::Failure,
        <Counter as ReasoningTokenCounter>::Failure,
    >,
>;
pub fn chat_call<'call, Config, Client, Counter>(
    call: ChatCall<'call>,
    mut deps: ChatDeps<'call, Config, Client, Counter>,
) -> LocalBoxFuture<'call, ChatResult<Client, Counter>>
where
    Config: ChatConfig,
    Client: ChatTransport,
    Counter: ReasoningTokenCounter + Unpin + Send + 'call,
{
    async move {
        if call.style == CallStyle::Embedding {
            return Ok(ChatOutcome::Embedding);
        }
        let prepared = prepare::prepare(&call, &mut deps).await?;
        if prepared.options.stream {
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
            return open_stream(
                deps.clients,
                deps.stream_logger,
                build,
                facts,
                call.style == CallStyle::Async,
            )
            .await;
        }
        send::buffered(call, prepared, deps).await
    }
    .boxed_local()
}
