use crate::operation::OperationError;
use betterlite_anthropic::completion::error::CompletionError;
use http::header::InvalidHeaderName;
use http::header::InvalidHeaderValue;
use std::error::Error as StdError;
pub mod config;
pub mod legacy;
mod messages;
pub mod preparation;
pub(crate) mod tests;

use betterlite_anthropic::anthropic_http_stream::call::StreamLogger;
use betterlite_anthropic::chat_call::ChatOutcome;
use betterlite_anthropic::chat_call::error::ChatCallError;
use betterlite_anthropic::chat_call::transport::ChatTransport;
use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_anthropic::request::error::RequestError;
use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_core::api::anthropic_beta_config::matrix::BetaMatrix;
use betterlite_http::client_kind::ClientKind;
use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::anthropic_results::tool_result::FileBlocks;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_tokenizer::engine::{Engine, error::EngineError};
use chrono::{DateTime, Utc};

use super::CallLogging;
use super::base::Surface;
use super::completion::{CompletionHandler, HandlerFuture, ResolvedCall};
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
    ClientFailure: StdError + Send + Sync + 'static,
    FileFailure: StdError + Send + Sync + 'static,
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
    Legacy(#[from] CompletionError),
    #[error(transparent)]
    Http(#[from] OperationError),
    #[error("a completion record cannot be converted")]
    Json(#[from] serde_json::Error),
    #[error("the completion endpoint is invalid")]
    Url(#[from] url::ParseError),
    #[error("a completion header is invalid")]
    HeaderName(#[from] InvalidHeaderName),
    #[error("a completion header is invalid")]
    HeaderValue(#[from] InvalidHeaderValue),
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
        call: ResolvedCall<ClientHandle<Client>, Engine>,
        log: &'call mut CallLogging<'_, '_>,
    ) -> HandlerFuture<'call, Self::Reply, Self::Failure> {
        match call.surface {
            Surface::Legacy => Box::pin(async move {
                legacy::complete(
                    &call.call,
                    self.runtime.pooled,
                    &self.legacy,
                    self.runtime.now,
                )
                .await
                .map(Reply::Legacy)
            }),
            Surface::Messages => messages::complete(self, call, log),
        }
    }

    fn logged_response(reply: &Self::Reply) -> Result<Json, Self::Failure> {
        let value = match reply {
            Reply::Messages(ChatOutcome::Reply(envelope)) => serde_json::to_value(envelope)?,
            Reply::Legacy(legacy::Reply::Buffered(envelope)) => serde_json::to_value(envelope)?,
            Reply::Messages(ChatOutcome::Stream(stream)) => serde_json::to_value(StreamLog {
                model: &stream.model,
                custom_llm_provider: stream.custom_llm_provider,
            })?,
            Reply::Messages(ChatOutcome::Embedding)
            | Reply::Legacy(legacy::Reply::Stream { .. }) => serde_json::Value::Null,
        };
        Ok(serde_json::from_value(value)?)
    }
}

#[derive(serde::Serialize)]
struct StreamLog<'reply> {
    model: &'reply str,
    custom_llm_provider: &'reply str,
}
