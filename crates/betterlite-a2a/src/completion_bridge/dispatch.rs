use std::error::Error;
use std::future::Future;

use betterlite_core::agents::json::AgentJson;
use futures_util::StreamExt as _;
use futures_util::stream::{BoxStream, iter};
use thiserror::Error;

use super::completion::{CompletionChunk, CompletionPort};
use super::params::{BridgeCall, ParamsError, build_completion_params, optional_text};
use super::streaming::{EventContent, stream_event};
use super::{A2AStreamingContext, EventSource, response_to_a2a};
use crate::provider_selector::{A2AProviderConfig, get_provider_config};
use crate::watsonx_run::accumulator::MAX_TEXT_BYTES;

mod buffered;
mod streamed;
mod tests;

pub type BridgeEvents<'stream, E> = BoxStream<'stream, Result<AgentJson, E>>;

#[derive(Debug, Error)]
pub enum CompletionError<C>
where
    C: Error + 'static,
{
    #[error(transparent)]
    Params(#[from] ParamsError),
    #[error("the A2A completion call failed")]
    Completion(#[source] C),
    #[error("the A2A streamed answer text passed {max} bytes")]
    TextTooLong { max: usize },
    #[error("the A2A completion event could not be encoded")]
    Encode(#[source] serde_json::Error),
}

#[derive(Debug, Error)]
pub enum BridgeError<C, P>
where
    C: Error + 'static,
    P: Error + 'static,
{
    #[error(transparent)]
    Completion(#[from] CompletionError<C>),
    #[error("the A2A provider config failed")]
    Provider(#[source] P),
}

// LiteLLM returns the provider config instance; the registry that owns the configs is B142's
pub trait A2AProviderDispatch: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn handle_non_streaming(
        &self,
        config: A2AProviderConfig,
        call: &BridgeCall<'_>,
    ) -> impl Future<Output = Result<AgentJson, Self::Error>> + Send;

    fn handle_streaming(
        &self,
        config: A2AProviderConfig,
        call: &BridgeCall<'_>,
    ) -> impl Future<Output = Result<BridgeEvents<'static, Self::Error>, Self::Error>> + Send;
}

// LiteLLM's handle_non_streaming and handle_streaming differ only in how the answer is delivered
pub struct Buffered;

pub struct Streamed;

pub trait Mode<'call, C, P>
where
    C: CompletionPort,
    P: A2AProviderDispatch,
{
    type Output;

    fn provider(
        providers: &'call P,
        config: A2AProviderConfig,
        call: &'call BridgeCall<'_>,
    ) -> impl Future<Output = Self::Output> + Send + 'call;

    fn direct<S>(
        call: &'call BridgeCall<'_>,
        completion: &'call C,
        source: &'call mut S,
    ) -> impl Future<Output = Self::Output> + Send + 'call
    where
        S: EventSource + Send + 'call;

    fn failed(error: CompletionError<C::Error>) -> Self::Output;
}

const WORKING_STATUS: EventContent<'static> = EventContent::Status {
    state: "working",
    final_event: false,
    message: Some("Processing request..."),
};

const COMPLETED_STATUS: EventContent<'static> = EventContent::Status {
    state: "completed",
    final_event: true,
    message: None,
};

// LiteLLM's A2ACompletionBridgeHandler: select the provider config, or run the completion itself
pub async fn dispatch<'call, M, C, P, S>(
    call: &'call BridgeCall<'_>,
    completion: &'call C,
    providers: &'call P,
    source: &'call mut S,
) -> M::Output
where
    M: Mode<'call, C, P>,
    C: CompletionPort,
    P: A2AProviderDispatch,
    S: EventSource + Send + 'call,
{
    let config = match select_config(call) {
        Ok(config) => config,
        Err(error) => return M::failed(CompletionError::Params(error)),
    };
    match config {
        Some(selected) => M::provider(providers, selected, call).await,
        None => M::direct(call, completion, source).await,
    }
}

pub async fn handle_non_streaming_direct<C, S>(
    call: &BridgeCall<'_>,
    completion: &C,
    source: &mut S,
) -> Result<AgentJson, CompletionError<C::Error>>
where
    C: CompletionPort,
    S: EventSource,
{
    let built = build_completion_params(call, false)?;
    let response = completion
        .complete(built)
        .await
        .map_err(CompletionError::Completion)?;
    let converted = response_to_a2a(Some(&response), Some(call.request_id), source);
    serde_json::to_value(converted).map_err(CompletionError::Encode)
}

#[must_use]
pub fn handle_streaming_direct<'call, C, S>(
    call: &BridgeCall<'_>,
    completion: &'call C,
    mut source: S,
) -> BridgeEvents<'call, CompletionError<C::Error>>
where
    C: CompletionPort,
    S: EventSource + Send + 'call,
{
    let built = match build_completion_params(call, true) {
        Ok(built) => built,
        Err(error) => return iter(Some(Err(CompletionError::Params(error)))).boxed(),
    };
    let ctx = A2AStreamingContext::new(
        call.request_id.to_owned(),
        call.params.message.clone(),
        &mut source,
    );
    async_stream::stream! {
        yield Ok(stream_event(&ctx, EventContent::Task, &mut source));
        yield Ok(stream_event(&ctx, WORKING_STATUS, &mut source));
        let mut chunks = match completion.stream(built).await {
            Ok(chunks) => chunks,
            Err(error) => {
                yield Err(CompletionError::Completion(error));
                return;
            }
        };
        let mut text = String::new();
        while let Some(next) = chunks.next().await {
            if let Err(error) = accumulate(&mut text, next) {
                yield Err(error);
                return;
            }
        }
        if !text.is_empty() {
            yield Ok(stream_event(&ctx, EventContent::Artifact(&text), &mut source));
        }
        yield Ok(stream_event(&ctx, COMPLETED_STATUS, &mut source));
    }
    .map(|event| serde_json::to_value(event?).map_err(CompletionError::Encode))
    .boxed()
}

fn accumulate<C>(
    text: &mut String,
    next: Result<CompletionChunk, C>,
) -> Result<(), CompletionError<C>>
where
    C: Error + 'static,
{
    let chunk = next.map_err(CompletionError::Completion)?;
    let content = chunk.first_content().unwrap_or_default();
    if text.len().saturating_add(content.len()) > MAX_TEXT_BYTES {
        return Err(CompletionError::TextTooLong {
            max: MAX_TEXT_BYTES,
        });
    }
    text.push_str(content);
    Ok(())
}

fn select_config(call: &BridgeCall<'_>) -> Result<Option<A2AProviderConfig>, ParamsError> {
    let provider = optional_text(call.litellm_params, "custom_llm_provider")?;
    let model = optional_text(call.litellm_params, "model")?;
    Ok(get_provider_config(provider, model))
}
