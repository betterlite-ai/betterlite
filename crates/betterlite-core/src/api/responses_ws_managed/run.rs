use std::error::Error;

use async_trait::async_trait;
use thiserror::Error;

use super::process::{ManagedTurnError, process_response_create};
use super::provider::ResponsesStreaming;
use crate::api::responses_ws_history::serialization::send_error;
use crate::api::responses_ws_history::{FrameSink, ManagedResponsesWebSocket};

#[async_trait]
pub trait FrameSource: Send {
    type Error: Error + Send + Sync + 'static;

    async fn receive_text(&mut self) -> Result<String, FrameReceiveError<Self::Error>>;
}

// Every receive failure ends the connection quietly, so run only needs the cause to log it.
#[derive(Debug, Error)]
pub enum FrameReceiveError<Source> {
    #[error("{0}")]
    Disconnected(#[source] Source),
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
