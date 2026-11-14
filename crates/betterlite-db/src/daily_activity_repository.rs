pub mod database;
pub mod export;
pub mod metadata;
pub mod query;
pub mod rows;

use crate::daily_activity_export::DailyActivityExportError;
use crate::daily_activity_sql::DailyActivitySqlError;
use crate::postgres_store::errors::DbError;
use rows::RowError;

pub const USAGE_KEY_SEARCH_MAX: i64 = 100;
pub const USAGE_MODEL_TOP_KEYS_MAX: i64 = 100;
pub const USAGE_CACHE_LEAKAGE_KEYS_MAX: i64 = 100;
pub const USAGE_EXPORT_BATCH_SIZE: usize = 1000;

pub struct DailyActivityRepository<Database, Reads, Clock> {
    pub database: Database,
    pub proxy_reads: Reads,
    pub clock: Clock,
}

#[derive(Debug, thiserror::Error)]
pub enum DailyActivityRepositoryError {
    #[error(transparent)]
    Database(#[from] DbError<RowError>),
    #[error(transparent)]
    Row(#[from] RowError),
    #[error(transparent)]
    Sql(#[from] DailyActivitySqlError),
    #[error(transparent)]
    Export(#[from] DailyActivityExportError),
    #[error("limit must be between 1 and {maximum}")]
    Limit { maximum: i64 },
    #[error("pagination arithmetic is out of range")]
    Pagination,
}

pub type Result<T> = std::result::Result<T, DailyActivityRepositoryError>;

mod tests;
