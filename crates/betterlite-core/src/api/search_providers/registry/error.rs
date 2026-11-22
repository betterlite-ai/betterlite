use std::error::Error as StdError;
use thiserror::Error;
// the erased config has one error type, so each provider's own error travels boxed
#[derive(Debug, Error)]
pub enum SearchConfigError {
    #[error(transparent)]
    Provider(Box<dyn StdError + Send + Sync>),
    #[error("the search request body was not built before the provider hook that needs it ran")]
    MissingBody,
    #[error("the search request body could not be encoded as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}
impl SearchConfigError {
    pub(crate) fn provider(error: impl StdError + Send + Sync + 'static) -> Self {
        Self::Provider(Box::new(error))
    }
}
