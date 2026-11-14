use std::time::Duration;

use tokio::time::timeout;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use crate::postgres_store::errors::{DbError, backend};
use crate::postgres_store::session::PostgresStore;

pub const STATEMENT_TIMEOUT_MILLIS: u64 = 5_000;
pub const TRANSACTION_TIMEOUT_MILLIS: u64 = 10_000;
const STATEMENT_TIMEOUT_SQL: &str = "SET LOCAL statement_timeout = 5000";

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
    let read = read_rows(store, sql, params, planner_settings);
    match timeout(Duration::from_millis(TRANSACTION_TIMEOUT_MILLIS), read).await {
        Ok(rows) => rows.map_err(|source| BoundedReadError::Database { source }),
        Err(_) => Err(BoundedReadError::TransactionTimeout {
            millis: TRANSACTION_TIMEOUT_MILLIS,
        }),
    }
}

async fn read_rows(
    store: &PostgresStore,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
    planner_settings: &[&str],
) -> Result<Vec<Row>, DbError> {
    let mut client = store.session().await?;
    let transaction = client.transaction().await.map_err(backend)?;
    transaction
        .execute(STATEMENT_TIMEOUT_SQL, &[])
        .await
        .map_err(backend)?;
    for setting in planner_settings {
        transaction.execute(*setting, &[]).await.map_err(backend)?;
    }
    transaction.query(sql, params).await.map_err(backend)
}
