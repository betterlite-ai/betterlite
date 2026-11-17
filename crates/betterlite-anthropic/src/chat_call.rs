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
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::anthropic_http_stream::call::{StreamBuild, StreamLogger};
use crate::anthropic_http_stream::completion::WrappedStream;
use crate::credentials::AnthropicSettings;
use crate::response::Logging;
use crate::response::wire::ResponseEnvelope;
use crate::stream_state::state::IdSource;
use crate::usage::ReasoningTokenCounter;

use config::ChatConfig;
use error::ChatCallError;
use prepare::Prepared;
use send::BufferedCall;
use send::streams::{StreamFacts, open_stream};
use transport::{ChatTransport, ClientKind, SuppliedChatClient};

// embedding() is an empty pass body: it builds no request, sends none and returns None
pub const EMBEDDING_REPLY: Option<ResponseEnvelope> = None;

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
    pub acompletion: bool,
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

// LiteLLM's AnthropicChatCompletion.completion around the selected config: prepare, filter, log, then send
pub async fn chat_call<Config, Client, Counter>(
    call: ChatCall<'_>,
    mut deps: ChatDeps<'_, Config, Client, Counter>,
) -> Result<ChatOutcome<Client::Failure, Counter>, ChatCallError<Client::Failure, Counter::Failure>>
where
    Config: ChatConfig,
    Client: ChatTransport,
    Counter: ReasoningTokenCounter + Unpin + Send,
{
    let prepared = prepare::prepare(&call, &mut deps).await?;
    if prepared.options.stream {
        return send_stream(call, prepared, deps).await;
    }
    // async takes any supplied client; sync keeps only an HTTPHandler and otherwise opens its own
    let client = match deps.clients.supplied.as_ref() {
        Some(supplied) if call.acompletion || supplied.kind == ClientKind::Sync => {
            supplied.transport
        }
        _ => deps.clients.pooled,
    };
    send::buffered(
        BufferedCall {
            call,
            body: prepared.body,
            header_map: prepared.header_map,
            optional_params: prepared.optional_params,
            options: prepared.options,
        },
        client,
        deps.config,
        Logging {
            state: &mut *deps.state,
            env: deps.env,
        },
        &deps.counter,
    )
    .await
}

// the stream branch: acompletion_stream_function takes only an AsyncHTTPHandler, make_sync_call any supplied client
async fn send_stream<Config, Client, Counter>(
    call: ChatCall<'_>,
    prepared: Prepared,
    deps: ChatDeps<'_, Config, Client, Counter>,
) -> Result<ChatOutcome<Client::Failure, Counter>, ChatCallError<Client::Failure, Counter::Failure>>
where
    Client: ChatTransport,
    Counter: ReasoningTokenCounter + Unpin + Send,
{
    let Prepared {
        body,
        header_map,
        optional_params,
        options,
    } = prepared;
    let ChatDeps {
        clients,
        stream_logger,
        counter,
        ids,
        max_event_bytes,
        max_error_body_bytes,
        ..
    } = deps;
    // the stream helper takes the messages as one JSON list, the way LiteLLM passes its copied list
    let messages = Json::List(call.messages);
    let facts = StreamFacts {
        model: call.model,
        api_base: call.api_base,
        messages: &messages,
        data: body,
        headers: header_map,
        timeout: call.timeout,
        json_mode: options.json_mode,
        optional_params: &optional_params,
        litellm_params: &call.litellm_params,
    };
    let build = StreamBuild {
        ids,
        counter,
        max_event_bytes,
        max_error_body_bytes,
    };
    open_stream(clients, stream_logger, build, facts, call.acompletion).await
}
