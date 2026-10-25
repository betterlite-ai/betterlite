use std::io;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum FixtureError {
    #[error("POSTGRES_TEST_URL is set but empty")]
    EmptyConfiguredUrl,
    #[error("POSTGRES_TEST_URL is not a PostgreSQL URL")]
    InvalidConfiguredUrl,
    #[error("configured PostgreSQL server is unreachable: {0}")]
    Connect(#[source] tokio_postgres::Error),
    #[error("testcontainers could not start PostgreSQL: {0}")]
    Container(#[source] testcontainers::TestcontainersError),
    #[error("fixture database operation failed: {0}")]
    Database(#[source] tokio_postgres::Error),
    #[error("fixture runtime failed: {0}")]
    Runtime(#[source] io::Error),
    #[error("fixture seed is invalid: {0}")]
    Seed(String),
    #[error("fixture database name is invalid")]
    DatabaseName,
    #[error("fixture database name was not created by this fixture")]
    ForeignDatabase,
}
