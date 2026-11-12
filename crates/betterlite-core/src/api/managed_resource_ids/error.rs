use std::str::Utf8Error;

use thiserror::Error;

use crate::api::api_boundaries::ApiLimitError;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ManagedResourceIdsError {
    #[error(transparent)]
    Limit(#[from] ApiLimitError),
    #[error("the managed resource id is not UTF-8")]
    NotUtf8 {
        #[source]
        source: Utf8Error,
    },
}
