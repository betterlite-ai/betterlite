mod adopt;
mod connect;
mod create;
mod database;
mod dispatch;
pub mod ensure;
mod leaf;
mod leaf_step;
mod locks;
mod partition;
pub mod replica_identity;
mod report;
mod tests;

use std::error::Error as StdError;

use crate::database_tls::options::ConnectionOptionsError;
use crate::migration_ledger::lock::JitterSource;
use crate::request_log_index_catalog::catalog::RequestLogIndexCatalogError;
use database::{DatabaseFailure, IndexDatabase};
use locks::RetryPause;
use tokio_postgres::error::SqlState;

#[derive(Debug, thiserror::Error)]
pub enum RequestLogIndexError {
    #[error("the request-log index database URL has no usable connection options: {source}")]
    ConnectionOptions {
        #[source]
        source: ConnectionOptionsError,
    },
    #[error("the request-log index connection pool could not be built: {source}")]
    Pool {
        #[source]
        source: deadpool_postgres::BuildError,
    },
    #[error("could not open the request-log index database connection: {source}")]
    Open {
        #[source]
        source: deadpool_postgres::PoolError,
    },
    #[error("timed out opening the request-log index database connection")]
    ConnectTimeout,
    #[error("the request-log index statement failed: {}", deepest_cause(.source))]
    Statement {
        #[from]
        source: tokio_postgres::Error,
    },
    #[error("the request-log index catalog query failed: {}", deepest_cause(.source))]
    Catalog {
        #[from]
        source: RequestLogIndexCatalogError,
    },
}

// tokio-postgres prints "db error" for a server error, so walk to the DbError that carries the message
fn deepest_cause(error: &(dyn StdError + 'static)) -> String {
    let mut cause = error;
    while let Some(deeper) = cause.source() {
        cause = deeper;
    }
    cause.to_string()
}

impl DatabaseFailure for RequestLogIndexError {
    fn is_database_failure(&self) -> bool {
        true
    }

    fn is_lock_not_available(&self) -> bool {
        match self {
            Self::Statement { source }
            | Self::Catalog {
                source: RequestLogIndexCatalogError::Query(source),
            } => source.code() == Some(&SqlState::LOCK_NOT_AVAILABLE),
            Self::ConnectionOptions { .. }
            | Self::Pool { .. }
            | Self::Open { .. }
            | Self::ConnectTimeout => false,
        }
    }
}

pub(crate) struct IndexDependencies<'dependencies> {
    pub(crate) jitter: &'dependencies dyn JitterSource,
    pub(crate) pause: &'dependencies dyn RetryPause,
}

pub(crate) struct IndexBuilder<'build, D>
where
    D: IndexDatabase,
{
    pub(crate) database: &'build D,
    pub(crate) schema: &'build str,
    pub(crate) dependencies: &'build IndexDependencies<'build>,
}
