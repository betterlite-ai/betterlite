use betterlite_analytics::canonical_date::ScopeDenied;

use crate::management_guards::error::ManagementGuardError;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UserActivityScopeError {
    #[error(transparent)]
    Caller(#[from] ManagementGuardError),
    #[error("{}", .0.reason)]
    Denied(ScopeDenied),
}
