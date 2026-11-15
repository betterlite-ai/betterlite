use serde::de::DeserializeOwned;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use crate::auxiliary_repository::repository::TableError;
use crate::budget_permission_rows::JsonData;
use crate::postgres_store::errors::backend;
use crate::postgres_store::session::PostgresStore;

pub(super) type Params<'query> = [&'query (dyn ToSql + Sync)];

fn decode_row(row: &Row) -> Result<JsonData, TableError> {
    let text: String = row
        .try_get(0)
        .map_err(|source| TableError::Database(backend(source)))?;
    serde_json::from_str(&text).map_err(TableError::Row)
}

pub(super) async fn query_one<R>(
    store: &PostgresStore,
    sql: &str,
    params: &Params<'_>,
) -> Result<Option<R>, TableError>
where
    R: DeserializeOwned,
{
    let client = store.session().await?;
    let row = client
        .query_opt(sql, params)
        .await
        .map_err(|source| TableError::Database(backend(source)))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let text: String = row
        .try_get(0)
        .map_err(|source| TableError::Database(backend(source)))?;
    serde_json::from_str(&text)
        .map(Some)
        .map_err(TableError::Row)
}

pub(super) async fn query_all<R>(
    store: &PostgresStore,
    sql: &str,
    params: &Params<'_>,
) -> Result<Vec<R>, TableError>
where
    R: DeserializeOwned,
{
    let client = store.session().await?;
    let rows = client
        .query(sql, params)
        .await
        .map_err(|source| TableError::Database(backend(source)))?;
    rows.iter().map(decode_row).collect()
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
