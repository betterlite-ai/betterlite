use std::error::Error as StdError;
use std::sync::{Mutex, PoisonError};

use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use tokio_util::sync::CancellationToken;

use super::end::ForwardEnd;
use super::error::ClientForwardError;
use crate::agentcore_stream::IdSource;
use crate::realtime_events::ack::{AckError, SessionAck};
use crate::realtime_events::client::{ClientMessages, RealtimeClientWebSocket};
use crate::realtime_events::decode::{decode_realtime_client_preface, json_str};
use crate::realtime_events::scope::EventScope;
use crate::realtime_events::stream::BedrockBidirectionalStream;
use crate::realtime_input::RealtimeInput;
use crate::realtime_input::frames::InputFrame;

pub struct ClientForward<'state, C, S>
where
    C: StdError + Send + Sync + 'static,
    S: StdError + Send + Sync + 'static,
{
    pub client: &'state dyn RealtimeClientWebSocket<Error = C>,
    pub stream: &'state dyn BedrockBidirectionalStream<Error = S>,
    // The provider direction dispatches against the same session, so both pumps share one input state
    pub shared: &'state Mutex<RealtimeInput>,
    pub scope: &'state mut EventScope,
    pub model: &'state str,
    pub trace_id: Option<&'state str>,
    pub ids: &'state dyn IdSource,
    pub redactor: &'state Redactor,
    pub initial: Option<String>,
    pub cancel: &'state CancellationToken,
}

impl<C, S> ClientForward<'_, C, S>
where
    C: StdError + Send + Sync + 'static,
    S: StdError + Send + Sync + 'static,
{
    // Cancellation and every failure leave through the same input close, as the source's finally does
    pub async fn forward_client_to_bedrock(&mut self) -> Result<ForwardEnd, ClientForwardError<S>> {
        // The token is copied out so the pump's own borrow stays free for the forwarding future
        let cancel = self.cancel;
        let outcome = cancel.run_until_cancelled(self.forward()).await;
        self.close_input().await;
        outcome.unwrap_or(Ok(ForwardEnd::Cancelled))
    }

    async fn forward(&mut self) -> Result<ForwardEnd, ClientForwardError<S>> {
        let client = self.client;
        let redactor = self.redactor;
        let mut messages: ClientMessages<'_, C> = ClientMessages {
            client,
            initial: self.initial.take(),
            redactor,
            done: false,
        };
        while let Some(message) = messages.next().await {
            tracing::debug!("Bedrock Realtime: Received from client");
            let parsed = decode_realtime_client_preface(message.as_bytes());
            let is_session_update = parsed.get("type").and_then(json_str) == Some("session.update");
            // The replay marker holds the message the transform never got to send
            if is_session_update {
                self.scope.pending_session_update = Json::Text(message.clone());
            }
            for frame in self.transformed(&message)? {
                self.send_frame(&frame).await?;
            }
            // False means the client is gone; the input close below still runs for it
            if is_session_update && !self.ack_session_update(&parsed).await? {
                break;
            }
        }
        Ok(ForwardEnd::Ended)
    }

    fn transformed(&self, message: &str) -> Result<Vec<InputFrame>, ClientForwardError<S>> {
        let mut input = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
        input
            .transform_realtime_request(message.as_bytes(), self.ids)
            .map_err(ClientForwardError::Input)
    }

    async fn send_frame(&self, frame: &InputFrame) -> Result<(), ClientForwardError<S>> {
        let event = frame_json::<S>(frame)?;
        self.stream.input_stream().send(event).await?;
        tracing::debug!("Bedrock Realtime: Sent to Bedrock");
        Ok(())
    }

    async fn ack_session_update(
        &mut self,
        parsed: &IndexMap<String, Json>,
    ) -> Result<bool, AckError<S>> {
        let mut ack: SessionAck<'_, C, S> = SessionAck {
            client: self.client,
            stream: self.stream,
            scope: &mut *self.scope,
            model: self.model,
            trace_id: self.trace_id,
            message: parsed,
            ids: self.ids,
            redactor: self.redactor,
        };
        ack.ack_session_update().await
    }

    async fn close_input(&self) {
        let frames = {
            let mut input = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
            input.session_close_messages()
        };
        for frame in frames {
            let Ok(event) = frame_json::<S>(&frame) else {
                continue;
            };
            // One closing frame's failure never skips the rest, and never replaces the pump's own error
            let _sent = self.stream.input_stream().send(event).await;
        }
        let _closed = self.stream.input_stream().close().await;
    }
}

// The transform builds typed frames, and the stream takes the protocol's own JSON value
fn frame_json<S>(frame: &InputFrame) -> Result<Json, ClientForwardError<S>>
where
    S: StdError + 'static,
{
    serde_json::from_value(
        serde_json::to_value(frame).map_err(|source| ClientForwardError::Encode { source })?,
    )
    .map_err(|source| ClientForwardError::Encode { source })
}
