use serde::de::DeserializeOwned;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use crate::auxiliary_repository::repository::TableError;
use crate::postgres_store::errors::backend;
use crate::postgres_store::session::PostgresStore;

pub(super) type Params<'query> = [&'query (dyn ToSql + Sync)];

async fn fetch_optional(
    store: &PostgresStore,
    sql: &str,
    params: &Params<'_>,
) -> Result<Option<Row>, TableError> {
    let client = store.session().await?;
    client
        .query_opt(sql, params)
        .await
        .map_err(|source| TableError::Database(backend(source)))
}

fn decode_row<R>(row: &Row) -> Result<R, TableError>
where
    R: DeserializeOwned,
{
    let text: String = row
        .try_get(0)
        .map_err(|source| TableError::Database(backend(source)))?;
    serde_json::from_str(&text).map_err(TableError::Row)
}

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
        records.push(decode_row(row)?);
    }
    Ok(records)
}

pub(super) async fn query_one<R>(
    store: &PostgresStore,
    sql: &str,
    params: &Params<'_>,
) -> Result<Option<R>, TableError>
where
    R: DeserializeOwned,
{
    fetch_optional(store, sql, params)
        .await?
        .map_or_else(|| Ok(None), |row| decode_row(&row).map(Some))
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
