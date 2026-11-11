mod connect;
mod database;
pub mod ensure;
mod leaf;
mod locks;
mod partition;
pub mod replica_identity;
mod tests;

use crate::database_tls::options::ConnectionOptionsError;
use crate::migration_ledger::lock::JitterSource;
use crate::request_log_index_catalog::catalog::RequestLogIndexCatalogError;
use database::{DatabaseFailure, IndexDatabase};
use locks::RetryPause;
use tokio_postgres::error::SqlState;

#[derive(Debug, thiserror::Error)]
pub enum RequestLogIndexError {
    #[error("the request-log index database URL has no usable connection options")]
    ConnectionOptions {
        #[source]
        source: ConnectionOptionsError,
    },
    #[error("the request-log index connection pool could not be built")]
    Pool {
        #[source]
        source: deadpool_postgres::BuildError,
    },
    #[error("could not open the request-log index database connection")]
    Open {
        #[source]
        source: deadpool_postgres::PoolError,
    },
    #[error("the request-log index statement failed")]
    Statement {
        #[source]
        source: tokio_postgres::Error,
    },
    #[error("the request-log index catalog query failed")]
    Catalog {
        #[source]
        source: RequestLogIndexCatalogError,
    },
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
            Self::ConnectionOptions { .. } | Self::Pool { .. } | Self::Open { .. } => false,
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
