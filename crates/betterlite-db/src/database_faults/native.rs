use crate::postgres_store::errors::DbError as NativeDbError;
use deadpool_postgres::PoolError;
use std::error::Error as _;
use std::io::Error as IoError;
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
    if matches!(error, NativeDbError::NoDbConnected) {
        return true;
    }
    if let NativeDbError::Pool { source } = error {
        return match &source.driver {
            PoolError::Timeout(_) | PoolError::Closed => true,
            PoolError::Backend(driver) => is_driver_transport(driver),
            PoolError::NoRuntimeSpecified | PoolError::PostCreateHook(_) => false,
        };
    }
    if let NativeDbError::Backend { source } = error {
        return is_driver_transport(&source.driver);
    }
    native_sqlstate(error)
        .is_some_and(|code| classify_sqlstate(&code) == NativeFault::ConnectionUnavailable)
}

fn is_driver_transport(error: &tokio_postgres::Error) -> bool {
    if let Some(code) = error.code() {
        return classify_sqlstate(code) == NativeFault::ConnectionUnavailable;
    }
    error.is_closed()
        || error
            .source()
            .is_some_and(|source| source.downcast_ref::<IoError>().is_some())
}

mod tests;
