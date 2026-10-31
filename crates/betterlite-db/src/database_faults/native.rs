use crate::postgres_store::errors::DbError as NativeDbError;
use deadpool_postgres::PoolError;
use tokio_postgres::error::SqlState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeFault {
    Deadlock,
    ReadOnlyTransaction,
    ConnectionUnavailable,
    Unclassified,
}

#[must_use]
pub fn native_sqlstate<E>(error: &NativeDbError<E>) -> Option<SqlState> {
    match error {
        NativeDbError::SqlState { code, .. } => Some(code.clone()),
        NativeDbError::UniqueViolation { .. } => Some(SqlState::UNIQUE_VIOLATION),
        NativeDbError::NoDbConnected
        | NativeDbError::Pool { .. }
        | NativeDbError::Backend { .. }
        | NativeDbError::InvalidRecord { .. }
        | NativeDbError::CreateNull => None,
    }
}

#[must_use]
pub fn classify_sqlstate(code: &SqlState) -> NativeFault {
    if *code == SqlState::T_R_DEADLOCK_DETECTED {
        NativeFault::Deadlock
    } else if *code == SqlState::READ_ONLY_SQL_TRANSACTION {
        NativeFault::ReadOnlyTransaction
    } else if *code == SqlState::CONNECTION_EXCEPTION
        || *code == SqlState::CONNECTION_FAILURE
        || *code == SqlState::CANNOT_CONNECT_NOW
        || *code == SqlState::ADMIN_SHUTDOWN
        || *code == SqlState::CRASH_SHUTDOWN
    {
        NativeFault::ConnectionUnavailable
    } else {
        NativeFault::Unclassified
    }
}

#[must_use]
pub fn is_native_transport_error<E>(error: &NativeDbError<E>) -> bool {
    match error {
        NativeDbError::NoDbConnected => true,
        NativeDbError::Pool { source } => {
            matches!(&source.driver, PoolError::Timeout(_) | PoolError::Closed)
        }
        NativeDbError::SqlState { code, .. } => {
            classify_sqlstate(code) == NativeFault::ConnectionUnavailable
        }
        NativeDbError::Backend { .. }
        | NativeDbError::UniqueViolation { .. }
        | NativeDbError::InvalidRecord { .. }
        | NativeDbError::CreateNull => false,
    }
}

mod tests;
