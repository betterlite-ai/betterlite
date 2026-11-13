use thiserror::Error;

use crate::identity::provider_id::ProviderId;
use crate::provider_defaults_a::url::UrlError;

#[derive(Debug, Error)]
pub enum ProviderDefaultsError {
    #[error("{provider} has no OpenAI-compatible default entry in provider_defaults_a")]
    UnknownProvider { provider: ProviderId },
    #[error("bedrock_mantle needs the endpoint policy from the Mantle input resolver")]
    MissingMantleEndpoint,
    #[error("api_base {api_base} is not a URL LiteLLM accepts")]
    InvalidApiBase {
        api_base: String,
        #[source]
        source: UrlError,
    },
}
