use std::num::ParseIntError;

use betterlite_models::call_helpers::generic_params::GenericParamsError;
use betterlite_models::headers::affinity::AffinityError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParamsError {
    #[error("max_retries is not a whole number: {value}")]
    MaxRetries {
        value: String,
        #[source]
        source: ParseIntError,
    },
    #[error("provider_affinity_header is not a valid header: {0}")]
    Affinity(#[from] AffinityError),
}

// the shared GenericLiteLLMParams failures as the management write's 400 detail
impl From<GenericParamsError> for ParamsError {
    fn from(failure: GenericParamsError) -> Self {
        match failure {
            GenericParamsError::MaxRetries { value, source } => Self::MaxRetries { value, source },
            GenericParamsError::Affinity(cause) => Self::Affinity(cause),
        }
    }
}
