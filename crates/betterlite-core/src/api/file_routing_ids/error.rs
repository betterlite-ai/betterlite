use std::str::Utf8Error;

use betterlite_error::proxy_exception::ProxyException;
use thiserror::Error;

use crate::api::api_boundaries::ApiLimitError;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FileRoutingIdsError {
    #[error(transparent)]
    Limit(#[from] ApiLimitError),
    #[error("the file routing id is not UTF-8")]
    NotUtf8 {
        #[source]
        source: Utf8Error,
    },
    #[error(
        "the unified batch id carries neither an llm_batch_id: nor a generic_response_id: field"
    )]
    MissingBatchMarker,
    #[error(transparent)]
    InvalidLimit(Box<ProxyException>),
}
