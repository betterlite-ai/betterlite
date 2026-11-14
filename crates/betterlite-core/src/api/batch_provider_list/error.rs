use std::num::ParseFloatError;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum BatchProviderListError {
    #[error(
        "batch listing does not support the provider {provider}; supported providers are azure, hosted_vllm, litellm_proxy, openai, vertex_ai and xai"
    )]
    UnsupportedProvider { provider: String },
    #[error("the timeout is not a number: {source}")]
    InvalidTimeout {
        #[source]
        source: ParseFloatError,
    },
    #[error("the timeout must be finite")]
    NonFiniteTimeout,
}
