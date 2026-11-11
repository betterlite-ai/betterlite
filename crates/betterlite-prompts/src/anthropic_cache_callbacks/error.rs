use thiserror::Error;

use crate::cache_placement::error::PlacementError;

#[derive(Debug, Error)]
pub enum CacheControlCallbackError {
    #[error("a prompt value does not match the prompt wire format")]
    Wire {
        #[source]
        source: serde_json::Error,
    },
    #[error("cache control placement failed")]
    Placement {
        #[source]
        source: PlacementError,
    },
}
