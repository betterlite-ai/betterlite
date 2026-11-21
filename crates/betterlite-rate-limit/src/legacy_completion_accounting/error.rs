use betterlite_auth::auth_limit_inputs::error::AuthLimitInputsError;
use betterlite_auth::resolved_key::auth::AuthError;

use crate::rate_limiters::legacy_rate_limit_counters::usage_cache::UsageCacheError;

// why a completion could not be counted out; the success and failure hooks log it, the post-call hook returns it
#[derive(Debug, thiserror::Error)]
pub enum AccountingError {
    #[error("the usage cache call for {key} failed")]
    Cache {
        key: String,
        #[source]
        source: UsageCacheError,
    },
    #[error("the batch write of the new counters failed")]
    BatchWrite(#[source] UsageCacheError),
    #[error("the request metadata has no user_api_key")]
    MissingUserApiKey,
    #[error("{key} holds a bare number, not a request counter")]
    NotACounter { key: String },
    #[error("the counter under {key} does not fit a 64-bit integer")]
    Overflow { key: String },
    #[error("the key auth value for the model limit lookup could not be built")]
    KeyAuth(#[source] AuthError),
    #[error("the per-model limit lookup failed")]
    ModelLimit(#[source] AuthLimitInputsError),
}
