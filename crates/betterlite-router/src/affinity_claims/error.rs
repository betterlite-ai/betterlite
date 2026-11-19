use betterlite_context::local_ttl_cache::CacheError;
use thiserror::Error;

use super::pin::PinError;
use crate::routing_value_store::StoreError;

#[derive(Debug, Error)]
pub enum AffinityError {
    #[error("the request metadata field {field} is not a string")]
    MetadataNotText { field: &'static str },
    #[error("the request metadata field {field} is not a boolean")]
    MetadataNotBool { field: &'static str },
    #[error("the request metadata field {field} is out of the ttl range")]
    TtlOutOfRange { field: &'static str },
    #[error(transparent)]
    Pin(#[from] PinError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Cache(#[from] CacheError),
    #[error("the stored affinity value is not a pin")]
    InvalidLocalValue,
}
