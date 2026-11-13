use super::chain::{Failure, Origin};
use crate::database_faults::is_database_connection_error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllowRequestsOnOutage {
    Boolean(bool),
    Text(String),
    Unsupported,
}

impl Default for AllowRequestsOnOutage {
    fn default() -> Self {
        Self::Boolean(false)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    #[error("general_settings.allow_requests_on_db_unavailable must be a boolean or a string")]
    UnsupportedValue,
}

pub fn allows_requests_during_outage(policy: &AllowRequestsOnOutage) -> Result<bool, PolicyError> {
    match policy {
        AllowRequestsOnOutage::Boolean(allow) => Ok(*allow),
        AllowRequestsOnOutage::Text(text) => Ok(text.trim().to_lowercase().as_str() == "true"),
        AllowRequestsOnOutage::Unsupported => Err(PolicyError::UnsupportedValue),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandleOutcome {
    Suppressed,
    Rethrown(Failure),
}

pub fn handle_db_exception(
    failure: &Failure,
    policy: &AllowRequestsOnOutage,
) -> Result<HandleOutcome, PolicyError> {
    let connection = match &failure.origin {
        Origin::Database(error) => is_database_connection_error(error.error_type),
        Origin::Os
        | Origin::AsyncTimeout
        | Origin::Asyncpg(_)
        | Origin::AsyncpgUnavailable(_)
        | Origin::Other
        | Origin::BaseException => false,
    };
    if connection && allows_requests_during_outage(policy)? {
        return Ok(HandleOutcome::Suppressed);
    }
    Ok(HandleOutcome::Rethrown(failure.clone()))
}
