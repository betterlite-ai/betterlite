use std::error::Error as StdError;

use betterlite_context::local_ttl_cache::CacheError;
use betterlite_db::auxiliary_repository::repository::TableError;

// every failure LiteLLM's ownership functions raise, plus the failures a Rust port cannot avoid
#[derive(Debug, thiserror::Error)]
pub enum ContainerOwnershipError {
    #[error("Unable to record container ownership: caller has no identity scope.")]
    NoIdentityScope,
    #[error("Forbidden")]
    Forbidden,
    #[error("Unable to track container")]
    ConflictingPurpose,
    #[error(transparent)]
    Table(#[from] TableError),
    #[error("the container ownership cache rejected the entry")]
    Cache(#[from] CacheError),
    #[error("the container ownership key could not be encoded")]
    Encode(#[from] serde_json::Error),
}

impl ContainerOwnershipError {
    // the status LiteLLM's HTTPException carries for the same failure
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::NoIdentityScope | Self::Forbidden => 403,
            Self::ConflictingPurpose | Self::Table(_) | Self::Cache(_) | Self::Encode(_) => 500,
        }
    }
}

// what a filtered list returns: the upstream call's own failure passes through untouched
#[derive(Debug, thiserror::Error)]
pub enum ContainerListError<Failure>
where
    Failure: StdError + 'static,
{
    #[error(transparent)]
    Ownership(#[from] ContainerOwnershipError),
    #[error(transparent)]
    Fetch(Failure),
    #[error("limit must be a non-negative integer, got {limit}")]
    InvalidLimit { limit: i64 },
}

impl<Failure> ContainerListError<Failure>
where
    Failure: StdError + 'static,
{
    // None means the upstream failure carries its own status and the route reads it there
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Ownership(error) => Some(error.status_code()),
            Self::Fetch(_) => None,
            Self::InvalidLimit { .. } => Some(400),
        }
    }
}
