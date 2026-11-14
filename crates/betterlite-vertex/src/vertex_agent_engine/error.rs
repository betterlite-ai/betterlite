use std::str::Utf8Error;

use thiserror::Error;

use crate::model_urls::error::VertexUrlError;
use crate::token_cache::error::CredentialCacheError;

#[derive(Debug, Error)]
pub enum AgentEngineError {
    // VertexAgentEngineError, the class get_error_class builds: the provider's raw status and text
    #[error("{message}")]
    Api { status_code: u16, message: String },
    #[error("vertex_project is required for Vertex Agent Engine; set vertex_project")]
    ProjectRequired,
    #[error("the request has no messages")]
    EmptyMessages,
    #[error("the {field} optional parameter must be a string")]
    FieldType { field: &'static str },
    #[error("the vertex location is not valid")]
    Location(#[source] VertexUrlError),
    #[error("the Vertex AI access token could not be minted")]
    Auth(#[source] CredentialCacheError),
    #[error("the Vertex Agent Engine response body is not UTF-8")]
    NotUtf8(u16, #[source] Utf8Error),
}
impl AgentEngineError {
    // LiteLLM raises a bare ValueError or IndexError here, which a proxy answers with a 500; the request is at fault
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Api { status_code, .. } => Some(*status_code),
            Self::NotUtf8(status, _) => Some(*status),
            Self::ProjectRequired
            | Self::EmptyMessages
            | Self::FieldType { .. }
            | Self::Location(_) => Some(400),
            Self::Auth(_) => None,
        }
    }
}
// LiteLLM's VertexAgentEngineError takes the response's own status and message, and the headers are dropped
#[must_use]
pub fn get_error_class(message: &str, status_code: u16) -> AgentEngineError {
    AgentEngineError::Api {
        status_code,
        message: message.to_owned(),
    }
}
