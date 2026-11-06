use crate::routing_probe_ipc::ProbeFailure;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OperationError {
    #[error("{message}")]
    InvalidRequest { message: String },
    #[error("{message}")]
    UnknownOperation { message: String },
    #[error("{message}")]
    SamplesExhausted { message: String },
}

impl OperationError {
    #[must_use]
    pub const fn kind(&self) -> ProbeFailure {
        match self {
            Self::InvalidRequest { .. } => ProbeFailure::InvalidRequest,
            Self::UnknownOperation { .. } => ProbeFailure::UnknownOperation,
            Self::SamplesExhausted { .. } => ProbeFailure::SamplesExhausted,
        }
    }
}
