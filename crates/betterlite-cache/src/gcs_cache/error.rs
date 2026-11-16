use std::str::Utf8Error;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum GcsCacheError {
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
