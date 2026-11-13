use std::time::TryFromFloatSecsError;

use http::header::InvalidHeaderValue;
use thiserror::Error;

use crate::langfuse_settings::LangfuseSettingsError;

#[derive(Debug, Error)]
pub enum LangfuseExporterError {
    #[error("could not read the langfuse export timeout")]
    Settings(#[from] LangfuseSettingsError),
    #[error("LANGFUSE_TIMEOUT='{raw}' is not a usable length of time")]
    UnusableTimeout {
        raw: String,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error("the langfuse export endpoint {endpoint:?} is not a valid URL")]
    Endpoint {
        endpoint: String,
        #[source]
        source: url::ParseError,
    },
    #[error("the langfuse public key holds a character an HTTP header cannot carry")]
    PublicKey(#[source] InvalidHeaderValue),
    #[error("the langfuse authorization header cannot be built")]
    Authorization(#[source] InvalidHeaderValue),
}
