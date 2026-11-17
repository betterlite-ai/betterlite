use betterlite_secret_managers::secret_lookup::error::SecretLookupError;
use thiserror::Error;

use crate::finops::pointfive::error::PointFiveUploadError;
use crate::finops::pointfive::payload::PayloadError;
use crate::logging_failure::FailureError;

#[derive(Debug, Error)]
pub enum PointFiveError {
    #[error(
        "pointfive logging requires an api key. Set POINTFIVE_API_KEY, or litellm_settings.pointfive_params.api_key in config.yaml"
    )]
    MissingApiKey,
    #[error("the pointfive secret could not be read: {0}")]
    Secret(#[from] SecretLookupError),
    #[error("the standard logging payload could not be redacted: {0}")]
    Payload(#[from] FailureError),
    #[error("the pointfive batch could not be encoded: {0}")]
    Serialize(#[from] PayloadError),
    #[error(transparent)]
    Upload(#[from] PointFiveUploadError),
}
