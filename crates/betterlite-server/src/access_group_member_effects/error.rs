use betterlite_auth::auth_cache::payload::PayloadError;
use betterlite_auth::auth_invalidation::error::InvalidationError;
use betterlite_auth::resolved_key::auth::AuthError;
use thiserror::Error;

use crate::access_group_read::protocols::error::AccessGroupStoreError;

#[derive(Debug, Error)]
pub enum MemberEffectsError {
    #[error(transparent)]
    Database(#[from] AccessGroupStoreError),
    #[error("the cached team object could not be read")]
    TeamPayload(#[source] PayloadError),
    #[error("the cached key object could not be read")]
    KeyAuth(#[source] AuthError),
    #[error("the cached key object could not be re-encoded")]
    KeyJson(#[source] serde_json::Error),
    #[error("the cached object could not be stored")]
    CacheWrite(#[source] InvalidationError),
}

#[derive(Debug, Error)]
#[error("the agent registry rejected the modified agent")]
pub struct RegistrationRejected;
