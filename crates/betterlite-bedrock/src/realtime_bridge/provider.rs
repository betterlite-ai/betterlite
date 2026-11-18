use std::error::Error as StdError;
use std::str::from_utf8;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_core::api::realtime_contracts::events::{
    RealtimeResponseTransformInput, RealtimeResponseTypedDict,
};
use betterlite_logging::python_json;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use tokio_util::sync::CancellationToken;

use super::end::ForwardEnd;
use super::error::ProviderForwardError;
use crate::agentcore_stream::IdSource;
use crate::realtime_events::client::RealtimeClientWebSocket;
use crate::realtime_events::dispatch::RealtimeEvents;
use crate::realtime_events::logging::should_log_event;
use crate::realtime_events::stream::BedrockBidirectionalStream;
use crate::realtime_input::RealtimeInput;

// The pump hands each loggable event to the caller before pulling the next one, as the source's generator does
#[async_trait]
pub trait LoggedEventSink: Send {
    async fn record(&mut self, event: Json);
}

pub struct ProviderForward<'state, C, S>
where
    C: StdError + Send + Sync + 'static,
    S: StdError + Send + Sync + 'static,
{
    pub stream: &'state dyn BedrockBidirectionalStream<Error = S>,
    pub client: &'state dyn RealtimeClientWebSocket<Error = C>,
    pub shared: &'state Mutex<RealtimeInput>,
    pub state: &'state mut RealtimeResponseTransformInput,
    pub logged: &'state mut dyn LoggedEventSink,
    pub logged_types: &'state LoggedEventTypes,
    pub ids: &'state dyn IdSource,
    pub redactor: &'state Redactor,
    pub cancel: &'state CancellationToken,
}

impl<C, S> ProviderForward<'_, C, S>
where
    C: StdError + Send + Sync + 'static,
    S: StdError + Send + Sync + 'static,
{
    // The output direction has no close of its own: only a normal provider end closes the client
    pub async fn forward_bedrock_to_client(
        &mut self,
    ) -> Result<ForwardEnd, ProviderForwardError<S>> {
        // The token is copied out so the pump's own borrow stays free for the forwarding future
        let cancel = self.cancel;
        match cancel.run_until_cancelled(self.forward()).await {
            Some(result) => result,
            None => Ok(ForwardEnd::Cancelled),
        }
    }

    async fn forward(&mut self) -> Result<ForwardEnd, ProviderForwardError<S>> {
        let (_metadata, output) = self.stream.await_output().await?;
        loop {
            let Some(chunk) = output.receive().await? else {
                tracing::debug!("Bedrock Realtime: Bedrock stream ended");
                self.client.close(1000, None).await;
                return Ok(ForwardEnd::Ended);
            };
            let Some(bytes) = chunk.value().and_then(|part| part.bytes()) else {
                continue;
            };
            if bytes.is_empty() {
                continue;
            }
            let text = from_utf8(bytes).map_err(|source| ProviderForwardError::Utf8 { source })?;
            tracing::debug!("Bedrock Realtime: Received from Bedrock");
            let response = self.transform(text)?;
            let RealtimeResponseTypedDict {
                response: value,
                current_output_item_id,
                current_response_id,
                current_delta_chunks,
                current_conversation_id,
                current_item_chunks,
                current_delta_type,
                session_configuration_request,
            } = response;
            // All seven fields change before the first of the response's events leaves
            *self.state = RealtimeResponseTransformInput {
                session_configuration_request,
                current_output_item_id,
                current_response_id,
                current_delta_chunks,
                current_item_chunks,
                current_conversation_id,
                current_delta_type,
            };
            let messages = match value {
                Json::List(items) => items,
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
                    vec![value]
                }
            };
            for message in messages {
                if !self.send_to_client(&message).await {
                    return Ok(ForwardEnd::Ended);
                }
                if self.should_log(&message) {
                    self.logged.record(message).await;
                }
            }
        }
    }

    fn transform(&self, text: &str) -> Result<RealtimeResponseTypedDict, ProviderForwardError<S>> {
        let mut input = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
        let mut events = RealtimeEvents {
            session: &mut input.session,
            ids: self.ids,
            redactor: self.redactor,
        };
        events
            .transform_realtime_response(text.as_bytes(), &*self.state)
            .map_err(ProviderForwardError::Transform)
    }

    async fn send_to_client(&self, message: &Json) -> bool {
        let text = python_json::dumps(message);
        match self.client.send_text(text).await {
            Ok(()) => {
                tracing::debug!("Bedrock Realtime: Sent to client");
                true
            }
            Err(error) => {
                let reported = self.redactor.redact_string(&error.message);
                tracing::debug!("Bedrock to client forwarding ended: {reported}");
                false
            }
        }
    }

    fn should_log(&self, message: &Json) -> bool {
        match message {
            Json::Object(fields) => should_log_event(self.logged_types, fields),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => false,
        }
    }
}
