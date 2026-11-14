use std::time::Duration;

use tokio::time::timeout;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use crate::postgres_store::errors::{DbError, backend};
use crate::postgres_store::session::PostgresStore;

pub const STATEMENT_TIMEOUT_MILLIS: u64 = 5_000;
pub const TRANSACTION_TIMEOUT_MILLIS: u64 = 10_000;

#[derive(Debug, thiserror::Error)]
pub enum BoundedReadError {
    #[error("the bounded read exceeded its {millis} millisecond transaction bound")]
    TransactionTimeout { millis: u64 },
    #[error("database error")]
    Database {
        #[from]
        source: DbError,
    },
}

pub async fn bounded_rows(
    store: &PostgresStore,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
    planner_settings: &[&str],
) -> Result<Vec<Row>, BoundedReadError> {
    let pending = read_rows(store, sql, params, planner_settings);
    let Ok(loaded) = timeout(Duration::from_millis(TRANSACTION_TIMEOUT_MILLIS), pending).await
    else {
        return Err(BoundedReadError::TransactionTimeout {
            millis: TRANSACTION_TIMEOUT_MILLIS,
        });
    };
    loaded.map_err(|source| BoundedReadError::Database { source })
}

async fn read_rows(
    store: &PostgresStore,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
    planner_settings: &[&str],
) -> Result<Vec<Row>, DbError> {
    let mut client = store.session().await?;
    let transaction = client.transaction().await.map_err(backend)?;
    let statement_timeout = format!("SET LOCAL statement_timeout = {STATEMENT_TIMEOUT_MILLIS}");
    transaction
        .execute(&statement_timeout, &[])
        .await
        .map_err(backend)?;
    for setting in planner_settings {
        transaction.execute(*setting, &[]).await.map_err(backend)?;
    }
    transaction.query(sql, params).await.map_err(backend)
}
