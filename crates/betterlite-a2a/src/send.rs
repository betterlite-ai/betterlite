mod bridge;
pub mod call;
pub mod deps;
pub mod error;
mod headers;
mod native;
mod params;
mod response;
mod routing;
mod streaming;
mod usage;

mod tests;

use betterlite_core::agents::provider::a2a::LiteLLMSendMessageResponse;
use betterlite_logging::logging_state::LoggingState;
use chrono::{DateTime, FixedOffset};
use futures_util::StreamExt as _;

use crate::completion_bridge::EventSource;
use crate::completion_bridge::completion::CompletionPort;
use crate::completion_bridge::dispatch::A2AProviderDispatch;
use crate::streaming_iterator::StreamSuccessDispatch;

use self::bridge::StreamedBridge;
use self::call::{SendCall, StreamCall};
use self::deps::{BridgeDeps, NativeDeps, StreamingDeps};
use self::error::{SendCallError, SendEntryError};

pub async fn asend_message<C, P, S>(
    call: SendCall,
    logging: Option<&mut LoggingState>,
    deps: &NativeDeps<'_>,
    bridge: &mut BridgeDeps<'_, C, P, S>,
) -> Result<LiteLLMSendMessageResponse, SendEntryError<C::Error, P::Error>>
where
    C: CompletionPort,
    P: A2AProviderDispatch,
    S: EventSource + Send,
{
    if let Some(provider) = routing::completion_provider(&call.litellm_params)? {
        return bridge::send_via_completion_bridge(&call, provider, bridge).await;
    }
    native::send_native(call, logging, deps)
        .await
        .map_err(SendEntryError::from)
}

pub async fn asend_message_streaming<'deps, C, P, S, Clock, Dispatch>(
    call: StreamCall,
    logging: Option<LoggingState>,
    deps: StreamingDeps<'deps, Clock, Dispatch>,
    bridge: &'deps mut BridgeDeps<'deps, C, P, S>,
) -> Result<StreamedBridge<'deps, C, P>, SendCallError>
where
    C: CompletionPort,
    P: A2AProviderDispatch,
    S: EventSource + Send + 'deps,
    Clock: FnMut() -> DateTime<FixedOffset> + Send + 'deps,
    Dispatch: StreamSuccessDispatch + 'deps,
{
    if let Some(provider) = routing::completion_provider(&call.litellm_params)? {
        tracing::info!(provider = %provider, "A2A streaming using completion bridge");
        return Ok(bridge::stream_via_completion_bridge(
            call,
            bridge.completion,
            bridge.providers,
            &mut *bridge.source,
        ));
    }
    let events = streaming::stream_native(call, logging, deps).await?;
    Ok(events
        .map(|event| event.map_err(SendEntryError::<C::Error, P::Error>::Call))
        .boxed())
}
