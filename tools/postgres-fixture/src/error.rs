use std::io;
use std::time::Duration;

use rustls::pki_types::pem::Error as PemError;
use tokio::task::JoinError;
use tokio::time::error::Elapsed;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum FixtureError {
    #[error("fixture provisioning task has already completed")]
    TaskCompleted,
    #[error("POSTGRES_TEST_URL is set but empty")]
    EmptyConfiguredUrl,
    #[error("POSTGRES_TEST_URL is not a PostgreSQL URL: {source}")]
    InvalidConfiguredUrl {
        #[source]
        source: tokio_postgres::Error,
    },
    #[error("configured PostgreSQL URL is invalid: {source}")]
    InvalidUrl {
        #[source]
        source: url::ParseError,
    },
    #[error("configured PostgreSQL URL has unknown sslmode {value}")]
    UnknownTlsMode { value: String },
    #[error("configured PostgreSQL URL explicitly requests unsupported sslmode=prefer")]
    ExplicitPreferTls,
    #[error("fixture field jwt_mappings is not supported")]
    UnsupportedJwtMappings,
    #[error("configured PostgreSQL server is unreachable: {0}")]
    Connect(#[source] tokio_postgres::Error),
    #[error("testcontainers could not start PostgreSQL: {0}")]
    Container(#[source] testcontainers::TestcontainersError),
    #[error("fixture database operation failed: {0}")]
    Database(#[source] tokio_postgres::Error),
    #[error("fixture connection driver failed: {0}")]
    Driver(#[source] tokio_postgres::Error),
    #[error("fixture TLS certificate PEM failed: {0}")]
    TlsPem(#[source] PemError),
    #[error("fixture TLS certificate was rejected: {0}")]
    TlsCertificate(#[source] rustls::Error),
    #[error("fixture native TLS certificate store failed: {0}")]
    TlsNative(#[source] rustls_native_certs::Error),
    #[error("fixture TLS has no trusted root certificates")]
    TlsNoRoots,
    #[error("fixture {operation} timed out after {duration:?}")]
    Timeout {
        operation: &'static str,
        duration: Duration,
        #[source]
        source: Elapsed,
    },
    #[error("fixture runtime failed: {0}")]
    Runtime(#[source] io::Error),
    #[error("duplicate {kind} identity at row {index}")]
    DuplicateIdentity { kind: &'static str, index: usize },
    #[error("{relation} references missing {kind} {identity}")]
    MissingReference {
        relation: &'static str,
        kind: &'static str,
        identity: String,
    },
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
    #[error("user {user_id} has conflicting roles")]
    ConflictingUserRole { user_id: String },
}
