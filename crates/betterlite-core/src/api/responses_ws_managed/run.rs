use async_trait::async_trait;
use thiserror::Error;

use super::process::{ManagedTurnError, process_response_create};
use super::provider::ResponsesStreaming;
use crate::api::responses_ws_history::serialization::send_error;
use crate::api::responses_ws_history::{FrameSink, ManagedResponsesWebSocket};

// the client side of the connection, which only run reads from
#[async_trait]
pub trait FrameSource: Send {
    async fn receive_text(&mut self) -> Result<String, FrameReceiveError>;
}

// litellm's websocket receive exception string, logged at debug before the connection ends quietly
#[derive(Debug, Error)]
#[error("{message}")]
pub struct FrameReceiveError {
    pub message: String,
}

pub async fn run<Sink, Provider, Source>(
    handler: &mut ManagedResponsesWebSocket<Sink>,
    provider: &Provider,
    mut source: Source,
) where
    Sink: FrameSink,
    Provider: ResponsesStreaming,
    Source: FrameSource,
{
    if let Err(error) = drive(handler, provider, &mut source).await {
        tracing::error!("ManagedResponsesWS: unexpected error: {error}");
        let message = format!("Internal server error: {error}");
        send_error(handler, &message, None).await;
    }
}

// first_message precedes the receive loop, and each turn finishes before the next frame is read
async fn drive<Sink, Provider, Source>(
    handler: &mut ManagedResponsesWebSocket<Sink>,
    provider: &Provider,
    source: &mut Source,
) -> Result<(), ManagedTurnError>
where
    Sink: FrameSink,
    Provider: ResponsesStreaming,
    Source: FrameSource,
{
    if let Some(first_message) = handler.first_message.clone() {
        process_response_create(handler, provider, &first_message).await?;
    }
    loop {
        let message = match source.receive_text().await {
            Ok(message) => message,
            Err(error) => {
                tracing::debug!("ManagedResponsesWS: client disconnected: {error}");
                break;
            }
        };
        process_response_create(handler, provider, &message).await?;
    }
    Ok(())
}
