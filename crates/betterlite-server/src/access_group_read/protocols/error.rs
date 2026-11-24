use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::postgres_store::errors::backend;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AccessGroupStoreError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error("record not found")]
    RecordNotFound,
}

impl From<tokio_postgres::Error> for AccessGroupStoreError {
    fn from(source: tokio_postgres::Error) -> Self {
        Self::Database(backend(source))
    }
}
