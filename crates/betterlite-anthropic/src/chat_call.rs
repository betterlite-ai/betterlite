pub mod config;
pub mod error;
pub mod prepare;
pub mod send;
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
// What the sync stream branch adds: the wrapper's provider, and whether make_sync_call receives the caller's
// speed and reverse map. The parent Anthropic handler passes them; Azure's Anthropic override passes neither.
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
        let facts = CallFacts {
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
        };
        finish(facts, prepared, deps).await
    }
    .boxed_local()
}
// The buffered send and the stream open every provider's completion runs once its own prepare returned
pub fn finish<'call, Config, Client, Counter>(
    facts: CallFacts<'call>,
    prepared: Prepared,
    deps: ChatDeps<'call, Config, Client, Counter>,
) -> LocalBoxFuture<'call, ChatResult<Client, Counter>>
where
    Config: ChatConfig,
    Client: ChatTransport,
    Counter: ReasoningTokenCounter + Unpin + Send + 'call,
{
    async move {
        if !prepared.stream {
            return send::buffered(facts, prepared, deps).await;
        }
        let messages = Json::List(facts.messages);
        let stream_facts = StreamFacts {
            model: facts.model,
            api_base: facts.api_base,
            messages: &messages,
            data: prepared.body,
            headers: prepared.header_map,
            timeout: facts.timeout,
            json_mode: prepared.json_mode,
            optional_params: &prepared.optional_params,
            litellm_params: &facts.litellm_params,
            custom_llm_provider: facts.sync_stream.custom_llm_provider,
            names: facts.sync_stream.names,
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
            stream_facts,
            facts.style == CallStyle::Async,
        )
        .await
    }
    .boxed_local()
}
