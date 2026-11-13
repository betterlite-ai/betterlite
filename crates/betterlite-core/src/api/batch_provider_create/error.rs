use crate::api::api_boundaries::ApiLimitError;
use crate::api::batch_provider_create::BatchProvider;

#[derive(Debug, thiserror::Error)]
pub enum BatchProviderCreateError {
    #[error("custom_llm_provider={provider} is not supported for create_batch")]
    UnsupportedProvider { provider: BatchProvider },
    // the provider adapter's own exception reduced to the status the caller branches on
    #[error("the provider batch call failed with status {status}")]
    Provider { status: u16 },
    #[error(transparent)]
    Limit(#[from] ApiLimitError),
}

impl BatchProviderCreateError {
    // the status LiteLLM's exception carries: BadRequestError for the unsupported provider
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::UnsupportedProvider { .. } => Some(400),
            Self::Provider { status } => Some(*status),
            Self::Limit(error) => Some(error.fields().status),
        }
    }
}
