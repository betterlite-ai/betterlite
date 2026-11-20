use betterlite_openai::batches::decode::BatchDecodeError;
use betterlite_openai::batches::error::SdkError;
use thiserror::Error;

use crate::endpoints::initialize::EndpointError;

#[derive(Debug, Error)]
pub enum AzureBatchesError {
    #[error(
        "OpenAI client is not initialized. Make sure api_key is passed or OPENAI_API_KEY is set in the environment."
    )]
    ClientMissing,
    #[error(
        "OpenAI client is not an instance of AsyncOpenAI. Make sure you passed an AsyncOpenAI client."
    )]
    NotAsync,
    #[error(
        "Azure client is not an instance of AsyncAzureOpenAI or AsyncOpenAI. Make sure you passed an async client."
    )]
    CancelNotAsync,
    #[error(
        "Azure client is not an instance of AzureOpenAI or OpenAI. Make sure you passed a sync client."
    )]
    NotSync,
    #[error(transparent)]
    Endpoint {
        #[from]
        source: EndpointError,
    },
    #[error(transparent)]
    Sdk {
        #[from]
        source: SdkError,
    },
    #[error("the SDK answer is not a batch: {source}")]
    Invalid {
        #[from]
        source: BatchDecodeError,
    },
}
