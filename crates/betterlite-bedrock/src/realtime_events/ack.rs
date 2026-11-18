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

#[derive(Debug, Error)]
pub enum AckError {
    #[error(transparent)]
    Stream(#[from] StreamError),
    #[error("the client session.modalities is not a list of strings")]
    Modalities,
    #[error("the session.updated event does not encode as JSON")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

pub struct SessionAck<'state> {
    pub client: &'state dyn RealtimeClientWebSocket,
    pub stream: &'state dyn BedrockBidirectionalStream,
    pub scope: &'state mut EventScope,
    pub model: &'state str,
    pub trace_id: Option<&'state str>,
    pub message: &'state IndexMap<String, Json>,
    pub ids: &'state dyn IdSource,
    pub redactor: &'state Redactor,
}

impl SessionAck<'_> {
    // The stream must accept the session before the scope says the session is committed to it
    pub async fn ack_session_update(&mut self) -> Result<bool, AckError> {
        self.stream.await_output().await?;
        self.scope.commit();
        let Some(trace_id) = self.trace_id else {
            return Ok(true);
        };
        let modalities = modalities_of(self.message)?;
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

// _json_dict(parsed.get("session")).get("modalities"), validated as list[str] | None
fn modalities_of(message: &IndexMap<String, Json>) -> Result<Option<Vec<String>>, AckError> {
    let session = message.get("session").and_then(json_dict);
    let Some(modalities) = session.and_then(|session| session.get("modalities")) else {
        return Ok(None);
    };
    match modalities {
        Json::Null => Ok(None),
        Json::List(items) => {
            let mut names = Vec::with_capacity(items.len());
            for item in items {
                let Json::Text(name) = item else {
                    return Err(AckError::Modalities);
                };
                names.push(name.clone());
            }
            Ok(Some(names))
        }
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            Err(AckError::Modalities)
        }
    }
}
