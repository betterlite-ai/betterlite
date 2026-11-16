use thiserror::Error;

// ast-grep-ignore: no-long-names
use crate::api::transcription_protocol::RealtimeTranscriptionProtocolError as ProtocolError;
use crate::api::vertex_chirp_protocol::ChirpProtocolError;
use crate::api::vertex_location::VertexLocationError;

#[derive(Debug, Error)]
pub enum ChirpAdapterError {
    #[error(transparent)]
    Protocol(#[from] ChirpProtocolError),
    #[error("Unsupported Speech-to-Text streaming model: {model}")]
    UnsupportedModel { model: String },
    #[error("session.update must configure the session before audio is sent")]
    UnconfiguredAudio,
    #[error(transparent)]
    Target(#[from] VertexLocationError),
    #[error("the Speech-to-Text streaming event is not a valid backend event")]
    Event {
        #[from]
        source: serde_json::Error,
    },
    #[error("the Speech-to-Text streaming response could not be encoded")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

impl From<ProtocolError> for ChirpAdapterError {
    fn from(error: ProtocolError) -> Self {
        Self::Protocol(ChirpProtocolError::from(error))
    }
}

impl ChirpAdapterError {
    // LiteLLM raises VertexAIError(400) for a bad project or location; the other failures are not replies
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Target(_) => Some(400),
            Self::Protocol(_)
            | Self::UnsupportedModel { .. }
            | Self::UnconfiguredAudio
            | Self::Event { .. }
            | Self::Encode { .. } => None,
        }
    }
}
