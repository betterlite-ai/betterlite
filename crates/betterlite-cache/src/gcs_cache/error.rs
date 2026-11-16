use std::str::Utf8Error;

use betterlite_logging::gcs_mock::settings::GcsMockConfigError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GcsCacheError {
    #[error("the GCS mock environment is invalid")]
    Mock {
        #[source]
        source: GcsMockConfigError,
    },
    #[error("the GCS cache body is not UTF-8")]
    Utf8 {
        #[source]
        source: Utf8Error,
    },
    #[error("the GCS cache body is not valid JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },
}
