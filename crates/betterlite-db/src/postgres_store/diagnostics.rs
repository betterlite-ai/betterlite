use std::fmt;
use tokio_postgres::error::SqlState;

// Stop generic source-chain logging before driver diagnostics expose parameter or row values.
#[derive(thiserror::Error)]
#[error("database SQLSTATE {code}: {constraint}",
    code = .driver.code().map_or("none", SqlState::code),
    constraint = .driver.as_db_error().and_then(|error| error.constraint()).unwrap_or("none")
)]
pub struct DriverError {
    pub driver: tokio_postgres::Error,
}

impl fmt::Debug for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

#[derive(thiserror::Error)]
#[error("database pool unavailable")]
pub struct PoolFailure {
    pub driver: deadpool_postgres::PoolError,
}

impl fmt::Debug for PoolFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
