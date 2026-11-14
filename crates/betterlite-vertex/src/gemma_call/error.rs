use std::error::Error as StdError;

use thiserror::Error;

use crate::endpoint_target::error::EndpointTargetError;
use crate::gemma_envelope::error::GemmaEnvelopeError;
use crate::gemma_envelope::post::PostFailure;

use super::reply::GemmaReplyError;

#[derive(Debug, Error)]
pub enum GemmaPlanError {
    #[error("api_base is required for Vertex AI Gemma models; provide the full endpoint URL")]
    MissingApiBase,
    #[error(transparent)]
    Target(#[from] EndpointTargetError),
}

impl GemmaPlanError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::MissingApiBase => 400,
            Self::Target(_) => 500,
        }
    }
}

#[derive(Debug, Error)]
pub enum GemmaCallError<ClientFailure, PooledFailure>
where
    ClientFailure: StdError + 'static,
    PooledFailure: StdError + 'static,
{
    #[error(transparent)]
    Plan(#[from] GemmaPlanError),
    #[error(transparent)]
    Envelope(#[from] GemmaEnvelopeError),
    #[error(transparent)]
    Reply(#[from] GemmaReplyError),
    #[error("the Gemma request body cannot be encoded as JSON")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    // the outer handler re-raises a status-bearing failure and turns every other one into a Vertex 500
    #[error("Request failed: {body}")]
    Status { status: u16, body: String },
    #[error(transparent)]
    Transport(#[from] PostFailure<ClientFailure, PooledFailure>),
}

impl<ClientFailure, PooledFailure> GemmaCallError<ClientFailure, PooledFailure>
where
    ClientFailure: StdError + 'static,
    PooledFailure: StdError + 'static,
{
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Plan(source) => source.status_code(),
            Self::Envelope(source) => source.status_code(),
            Self::Reply(source) => source.status_code(),
            Self::Encode { .. } | Self::Transport(_) => 500,
            Self::Status { status, .. } => *status,
        }
    }
}
