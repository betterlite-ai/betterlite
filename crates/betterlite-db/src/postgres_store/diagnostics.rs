use std::{error::Error, fmt};
use tokio_postgres::error::SqlState;

pub struct DriverError {
    pub driver: tokio_postgres::Error,
}

// Stop generic source-chain logging before driver diagnostics expose parameter or row values.
impl Error for DriverError {}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let constraint = self
            .driver
            .as_db_error()
            .and_then(|error| error.constraint());
        let code = self.driver.code().map_or("none", SqlState::code);
        write!(
            f,
            "database SQLSTATE {code}: {}",
            constraint.unwrap_or("none")
        )
    }
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
