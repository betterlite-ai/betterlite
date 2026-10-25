use std::io;
use std::time::Duration;

use tokio::task::JoinError;
use tokio::time::error::Elapsed;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum FixtureError {
    #[error("POSTGRES_TEST_URL is set but empty")]
    EmptyConfiguredUrl,
    #[error("POSTGRES_TEST_URL is not a PostgreSQL URL: {source}")]
    InvalidConfiguredUrl {
        #[source]
        source: tokio_postgres::Error,
    },
    #[error("configured PostgreSQL server is unreachable: {0}")]
    Connect(#[source] tokio_postgres::Error),
    #[error("testcontainers could not start PostgreSQL: {0}")]
    Container(#[source] testcontainers::TestcontainersError),
    #[error("fixture database operation failed: {0}")]
    Database(#[source] tokio_postgres::Error),
    #[error("fixture connection driver failed: {0}")]
    Driver(#[source] tokio_postgres::Error),
    #[error("fixture {operation} timed out after {duration:?}")]
    Timeout {
        operation: &'static str,
        duration: Duration,
        #[source]
        source: Elapsed,
    },
    #[error("fixture runtime failed: {0}")]
    Runtime(#[source] io::Error),
    #[error("fixture seed is invalid: {0}")]
    Seed(String),
    #[error("fixture connection driver task failed: {0}")]
    DriverJoin(#[source] JoinError),
    #[error("fixture setup failed and cleanup failed: {setup}; {cleanup}")]
    SetupCleanup {
        #[source]
        setup: Box<Self>,
        cleanup: Box<Self>,
    },
    #[error("multiple fixture cleanups failed: {first}; {second}")]
    MultipleCleanup {
        #[source]
        first: Box<Self>,
        second: Box<Self>,
    },
    #[error("fixture database name was not created by this fixture")]
    ForeignDatabase,
}
