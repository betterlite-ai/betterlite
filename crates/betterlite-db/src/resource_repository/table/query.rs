use serde::de::DeserializeOwned;
use tokio_postgres::types::ToSql;

use crate::auxiliary_repository::repository::TableError;
use crate::postgres_store::errors::backend;
use crate::postgres_store::session::PostgresStore;

pub(super) type Params<'query> = [&'query (dyn ToSql + Sync)];

// Every statement returns its rows as row_to_json text in the first column.
pub(super) async fn query_all<R>(
    store: &PostgresStore,
    sql: &str,
    params: &Params<'_>,
) -> Result<Vec<R>, TableError>
where
    R: DeserializeOwned,
{
    let rows = store
        .session()
        .await?
        .query(sql, params)
        .await
        .map_err(|source| TableError::Database(backend(source)))?;
    let mut records = Vec::with_capacity(rows.len());
    for row in &rows {
        let text: String = row
            .try_get(0)
            .map_err(|source| TableError::Database(backend(source)))?;
        records.push(serde_json::from_str(&text).map_err(TableError::Row)?);
    }
    Ok(records)
}

pub(super) async fn execute_count(
    store: &PostgresStore,
    sql: &str,
    params: &Params<'_>,
) -> Result<usize, TableError> {
    let client = store.session().await?;
    let rows = client
        .execute(sql, params)
        .await
        .map_err(|source| TableError::Database(backend(source)))?;
    usize::try_from(rows).map_err(|source| TableError::RowCount { source })
}
