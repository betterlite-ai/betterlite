pub mod config;
pub mod legacy;
pub mod preparation;
mod tests;

use std::sync::{Mutex, PoisonError};

use betterlite_anthropic::anthropic_http_stream::call::StreamLogger;
use betterlite_anthropic::chat_call::error::ChatCallError;
use betterlite_anthropic::chat_call::transport::{ChatTransport, ClientKind, SuppliedChatClient};
use betterlite_anthropic::chat_call::{
    CallStyle, ChatCall, ChatClients, ChatDeps, ChatOutcome, chat_call,
};
use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_anthropic::request::error::RequestError;
use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_core::api::anthropic_beta_config::matrix::BetaMatrix;
use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::anthropic_results::tool_result::FileBlocks;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_tokenizer::engine::{Engine, error::EngineError};
use chrono::{DateTime, Utc};
use secrecy::ExposeSecret as _;

use super::CallLogging;
use super::base::Surface;
use super::completion::{CompletionCall, CompletionHandler, HandlerFuture, ResolvedCall};
use config::Config;
use preparation::Preparation;

#[derive(Clone, Debug)]
pub struct ClientHandle<Client> {
    pub kind: ClientKind,
    pub transport: Client,
}

#[derive(Clone)]
pub struct Counter(Engine);
impl ReasoningTokenCounter for Counter {
    type Failure = EngineError;
    fn count_reasoning_tokens(&self, text: &str) -> Result<usize, EngineError> {
        self.0.encode(text).map(|tokens| tokens.len())
    }
}

pub struct Runtime<'deps, Client> {
    pub pooled: &'deps Client,
    pub settings: &'deps AnthropicSettings,
    pub matrix: &'deps BetaMatrix,
    pub stream_logger: &'deps dyn StreamLogger,
    pub ids: &'deps dyn IdSource,
    pub now: DateTime<Utc>,
    pub max_event_bytes: usize,
    pub max_error_body_bytes: usize,
    pub disable_add_prefix_to_prompt: bool,
    pub effort_used: bool,
}

pub struct Native<'deps, Client, Source, Metadata, Fetcher, Blocks, Ids> {
    pub runtime: Runtime<'deps, Client>,
    pub preparation: Preparation<'deps, Source, Metadata, Fetcher, Blocks, Ids>,
    pub legacy: legacy::Legacy<'deps>,
}

pub enum Reply<ClientFailure> {
    Messages(ChatOutcome<ClientFailure, Counter>),
    Legacy(legacy::Reply),
}

#[derive(Debug, thiserror::Error)]
pub enum Failure<ClientFailure, FileFailure>
where
    ClientFailure: std::error::Error + Send + Sync + 'static,
    FileFailure: std::error::Error + Send + Sync + 'static,
{
    #[error("the selected config is not an Anthropic Messages config")]
    Config,
    #[error("the resolved base is missing")]
    MissingBase,
    #[error(transparent)]
    Chat(Box<ChatCallError<ClientFailure, EngineError>>),
    #[error("Anthropic request preparation failed")]
    Prepare(#[source] Box<RequestError<FileFailure>>),
    #[error("extra_body must be an object")]
    ExtraBody,
    #[error(transparent)]
    Legacy(#[from] betterlite_anthropic::completion::error::CompletionError),
    #[error(transparent)]
    Http(#[from] crate::operation::OperationError),
    #[error("a completion record cannot be converted")]
    Json(#[from] serde_json::Error),
    #[error("the completion endpoint is invalid")]
    Url(#[from] url::ParseError),
    #[error("a completion header is invalid")]
    HeaderName(#[from] http::header::InvalidHeaderName),
    #[error("a completion header is invalid")]
    HeaderValue(#[from] http::header::InvalidHeaderValue),
}

impl<Client, Source, Metadata, Fetcher, Blocks, Ids> CompletionHandler<ClientHandle<Client>, Engine>
    for Native<'_, Client, Source, Metadata, Fetcher, Blocks, Ids>
where
    Client: ChatTransport + legacy::Transport,
    Source: DynamicMetadataSource,
    Metadata: ModelInfoClient,
    Fetcher: RemoteMediaFetcher + Sync,
    Blocks: FileBlocks + Sync,
    Ids: Fn() -> String + Sync,
{
    type Reply = Reply<Client::Failure>;
    type Failure = Failure<Client::Failure, Blocks::Failure>;

    fn complete<'call>(
        &'call mut self,
        resolved: ResolvedCall<ClientHandle<Client>, Engine>,
        log: &'call mut CallLogging<'_, '_>,
    ) -> HandlerFuture<'call, Self::Reply, Self::Failure> {
        Box::pin(async move {
            let mut call = resolved.call;
            match resolved.surface {
                Surface::Legacy => Ok(Reply::Legacy(
                    legacy::complete(&call, self.runtime.pooled, &self.legacy, self.runtime.now)
                        .await?,
                )),
                Surface::Messages => {
                    if resolved.config != Some(ChatConfigTag::Anthropic) {
                        return Err(Failure::Config);
                    }
                    let mut config = Config {
                        validated: Mutex::new(None),
                        prepared: None,
                        matrix: self.runtime.matrix,
                    };
                    let first = messages(&call, &config, &self.runtime, log).await;
                    let Some(validated) = config
                        .validated
                        .get_mut()
                        .unwrap_or_else(PoisonError::into_inner)
                        .take()
                    else {
                        return first.map(Reply::Messages);
                    };
                    let prepared = self
                        .preparation
                        .prepare(&call.model, &call.provider, validated)
                        .await
                        .map_err(|source| Failure::Prepare(Box::new(source)))?;
                    if !prepared.tool_name_reverse.is_empty() {
                        call.litellm_params.insert(
                            betterlite_anthropic::tool_names::REVERSE_MAP_KEY.to_owned(),
                            serde_json::from_value(serde_json::to_value(
                                &prepared.tool_name_reverse,
                            )?)?,
                        );
                    }
                    config.prepared = Some(prepared);
                    messages(&call, &config, &self.runtime, log)
                        .await
                        .map(Reply::Messages)
                }
            }
        })
    }

    fn logged_response(reply: &Self::Reply) -> Result<Json, Self::Failure> {
        let value = match reply {
            Reply::Messages(ChatOutcome::Reply(envelope)) => serde_json::to_value(envelope)?,
            Reply::Legacy(legacy::Reply::Buffered(envelope)) => serde_json::to_value(envelope)?,
            Reply::Messages(ChatOutcome::Stream(stream)) => {
                serde_json::json!({"model": stream.model, "custom_llm_provider": stream.custom_llm_provider})
            }
            Reply::Messages(ChatOutcome::Embedding)
            | Reply::Legacy(legacy::Reply::Stream { .. }) => serde_json::Value::Null,
        };
        Ok(serde_json::from_value(value)?)
    }
}

async fn messages<Client, FileFailure>(
    call: &CompletionCall<ClientHandle<Client>, Engine>,
    config: &Config<'_>,
    runtime: &Runtime<'_, Client>,
    log: &mut CallLogging<'_, '_>,
) -> Result<ChatOutcome<Client::Failure, Counter>, Failure<Client::Failure, FileFailure>>
where
    Client: ChatTransport,
    FileFailure: std::error::Error + Send + Sync + 'static,
{
    let native = ChatCall {
        model: &call.model,
        custom_llm_provider: &call.provider,
        messages: call.messages.clone(),
        optional_params: call.optional_params.clone(),
        litellm_params: call.litellm_params.clone(),
        caller_headers: call
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
            .collect(),
        api_base: call.api_base.as_deref().ok_or(Failure::MissingBase)?,
        api_key: call.api_key.as_ref(),
        settings: runtime.settings,
        timeout: call.timeout,
        style: if call.acompletion {
            CallStyle::Async
        } else {
            CallStyle::Sync
        },
        effort_used: runtime.effort_used,
        now: runtime.now.timestamp(),
        disable_add_prefix_to_prompt: runtime.disable_add_prefix_to_prompt,
        model_response: call.model_response.clone(),
    };
    let deps = ChatDeps {
        config,
        clients: ChatClients {
            pooled: runtime.pooled,
            supplied: call.client.as_ref().map(|client| SuppliedChatClient {
                kind: client.kind,
                transport: &client.transport,
            }),
        },
        state: log.state,
        env: log.env,
        stream_logger: runtime.stream_logger,
        counter: Counter(call.encoding.clone()),
        ids: runtime.ids,
        max_event_bytes: runtime.max_event_bytes,
        max_error_body_bytes: runtime.max_error_body_bytes,
    };
    chat_call(native, deps)
        .await
        .map_err(|source| Failure::Chat(Box::new(source)))
}
