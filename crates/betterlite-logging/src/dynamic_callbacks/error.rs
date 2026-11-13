use thiserror::Error;

use crate::logging_privacy::CallbackMetadataError;
use crate::registry::error::RegistryError;

#[derive(Debug, Error)]
pub enum DynamicCallbackError {
    #[error("dynamic callback '{callback}' is unavailable: {source}")]
    Unavailable {
        callback: String,
        #[source]
        source: RegistryError,
    },
    #[error(
        "the {list} dynamic callback list holds {count} callbacks, at most {limit} are allowed"
    )]
    TooMany {
        list: &'static str,
        count: usize,
        limit: usize,
    },
    #[error(transparent)]
    Params(#[from] CallbackMetadataError),
}
