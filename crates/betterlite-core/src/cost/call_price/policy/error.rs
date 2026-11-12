use std::error::Error as StdError;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum PolicyError {
    #[error("betterlite has no executable adapter for the {0} provider")]
    UnavailableAdapter(String),
    #[error("the {provider} provider could not price the call: {source}")]
    Failed {
        provider: String,
        #[source]
        source: Box<dyn StdError + Send + Sync>,
    },
}
