use std::error::Error;

use betterlite_env::EnvError;
use betterlite_logging::private_json::error::PrivateJsonError;

// One cache call's failure: the operation the manager logs and the cause it never raises.
#[derive(Debug, thiserror::Error)]
#[error("the health check cache {operation} failed: {source}")]
pub struct SharedHealthCacheError {
    pub operation: &'static str,
    pub source: Box<dyn Error + Send + Sync>,
}

// What perform_shared_health_check can raise: only the probe does, and the loop falls back on it.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SharedHealthError {
    #[error("the health check failed: {message}")]
    Probe { message: String },
}

// A stored value that is not a health check result: json.loads' failure and the row shape around it.
#[derive(Debug, thiserror::Error)]
pub enum CachedHealthError {
    #[error("the cached health check results are not valid JSON")]
    Private(#[from] PrivateJsonError),
    #[error("the cached health check results are not health check result rows")]
    Shape(#[from] serde_json::Error),
}

// The two TTLs read from DEFAULT_SHARED_HEALTH_CHECK_TTL and DEFAULT_SHARED_HEALTH_CHECK_LOCK_TTL.
#[derive(Debug, thiserror::Error)]
pub enum SharedHealthSettingsError {
    #[error(transparent)]
    Environment(#[from] EnvError),
}
