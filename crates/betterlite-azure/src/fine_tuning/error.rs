use betterlite_openai::fine_tuning::client::{BuildError, SdkError};
use betterlite_openai::fine_tuning::error::FineTuneError;
use thiserror::Error;

use crate::endpoints::initialize::EndpointError;

#[derive(Debug, Error)]
pub enum AzureFineTuneError {
    #[error(
        "Azure OpenAI client is not initialized. Make sure api_key is passed or AZURE_API_KEY is set in the environment."
    )]
    ClientMissing,
    #[error(
        "OpenAI client is not an instance of AsyncOpenAI. Make sure you passed an AsyncOpenAI client."
    )]
    NotAsyncClient,
    #[error(transparent)]
    Endpoint {
        #[from]
        source: EndpointError,
    },
    // the shared job layer keeps the SDK, build and decode failures; the Azure layer only adds its own client errors
    #[error(transparent)]
    Build {
        #[from]
        source: BuildError,
    },
    #[error(transparent)]
    Sdk {
        #[from]
        source: SdkError,
    },
    #[error(transparent)]
    Decode {
        #[from]
        source: serde_json::Error,
    },
}

impl From<FineTuneError> for AzureFineTuneError {
    fn from(error: FineTuneError) -> Self {
        match error {
            FineTuneError::ClientMissing => Self::ClientMissing,
            FineTuneError::NotAsyncClient => Self::NotAsyncClient,
            FineTuneError::Build { source } => Self::Build { source },
            FineTuneError::Sdk { source } => Self::Sdk { source },
            FineTuneError::Decode { source } => Self::Decode { source },
        }
    }
}
