use std::error::Error as StdError;

use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use thiserror::Error;

use super::client::RealtimeClientWebSocket;
use super::decode::json_dict;
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
        let modalities = match session.and_then(|session| session.get("modalities")) {
            None | Some(Json::Null) => None,
            Some(Json::List(items)) => {
                let mut names = Vec::with_capacity(items.len());
                for item in items {
                    let Json::Text(name) = item else {
                        return Err(AckError::Modalities);
                    };
                    names.push(name.clone());
                }
                Some(names)
            }
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
