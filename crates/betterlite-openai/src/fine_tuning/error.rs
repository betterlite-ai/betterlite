use thiserror::Error;

use super::client::{BuildError, SdkError};

#[derive(Debug, Error)]
pub enum FineTuneError {
    #[error(
        "OpenAI client is not initialized. Make sure api_key is passed or OPENAI_API_KEY is set in the environment."
    )]
    ClientMissing,
    #[error(
        "OpenAI client is not an instance of AsyncOpenAI. Make sure you passed an AsyncOpenAI client."
    )]
    NotAsyncClient,
    #[error("the SDK client could not be built")]
    Build {
        #[source]
        source: BuildError,
    },
    #[error("the SDK call failed: {source}")]
    Sdk {
        #[source]
        source: SdkError,
    },
    #[error("the fine-tuning job record is not valid")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
}
