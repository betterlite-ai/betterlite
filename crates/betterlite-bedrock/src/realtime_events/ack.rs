use std::error::Error as StdError;

use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use thiserror::Error;

use super::client::RealtimeClientWebSocket;
use super::decode::{json_dict, json_str};
use super::scope::EventScope;
use super::stream::{BedrockBidirectionalStream, StreamError};
use crate::agentcore_stream::IdSource;
use crate::realtime_session::RealtimeSession;
use crate::realtime_session::records::SessionEventType;

// The client's send failure ends the ack with False, so only the stream's error is a variant
#[derive(Debug, Error)]
pub enum AckError<S> {
    #[error(transparent)]
    Stream(#[from] StreamError<S>),
    #[error("the client session.modalities is not a list of strings")]
    Modalities,
    #[error("the session.updated event does not encode as JSON")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

pub struct SessionAck<'state, C, S>
where
    C: StdError + Send + Sync + 'static,
    S: StdError + Send + Sync + 'static,
{
    pub client: &'state dyn RealtimeClientWebSocket<Error = C>,
    pub stream: &'state dyn BedrockBidirectionalStream<Error = S>,
    pub scope: &'state mut EventScope,
    pub model: &'state str,
    pub trace_id: Option<&'state str>,
    pub message: &'state IndexMap<String, Json>,
    pub ids: &'state dyn IdSource,
    pub redactor: &'state Redactor,
}

impl<C, S> SessionAck<'_, C, S>
where
    C: StdError + Send + Sync + 'static,
    S: StdError + Send + Sync + 'static,
{
    // The stream must accept the session before the scope says the session is committed to it
    pub async fn ack_session_update(&mut self) -> Result<bool, AckError<S>> {
        self.stream.await_output().await?;
        self.scope.commit();
        let Some(trace_id) = self.trace_id else {
            return Ok(true);
        };
        // _json_dict(parsed.get("session")).get("modalities"), validated as list[str] | None
        let session = self.message.get("session").and_then(json_dict);
        let modalities = match session.and_then(|fields| fields.get("modalities")) {
            None | Some(Json::Null) => None,
            Some(Json::List(items)) => Some(
                items
                    .iter()
                    .map(|item| {
                        json_str(item)
                            .map(str::to_owned)
                            .ok_or(AckError::Modalities)
                    })
                    .collect::<Result<Vec<String>, _>>()?,
            ),
            Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
                return Err(AckError::Modalities);
            }
        };
        let event = RealtimeSession::session_event(
            SessionEventType::Updated,
            Some(self.model),
            trace_id,
            modalities.as_deref(),
            self.ids,
        );
        let text = serde_json::to_string(&event).map_err(|source| AckError::Encode { source })?;
        match self.client.send_text(text).await {
            Ok(()) => Ok(true),
            Err(error) => {
                let reported = self.redactor.redact_string(&error.message);
                tracing::debug!("Client to Bedrock forwarding ended: {reported}");
                Ok(false)
            }
        }
    }
}
