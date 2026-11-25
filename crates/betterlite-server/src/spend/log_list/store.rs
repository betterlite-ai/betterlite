use async_trait::async_trait;
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::postgres_store::session::PostgresStore;
use tokio_postgres::types::{ToSql, Type};

use crate::spend::log_results::projection::SpendLogListRow;
use crate::spend::sessions::params::{SessionParam, SessionQuery};

#[derive(Debug, thiserror::Error)]
pub enum ListStoreError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error("the spend log row could not be decoded")]
    Row(#[source] serde_json::Error),
}

// the two reads the list executes itself: the bounded count and the page
#[async_trait]
pub trait LogListStore: Send + Sync {
    async fn count_matches(&self, query: &SessionQuery) -> Result<i64, ListStoreError>;
    async fn list_rows(&self, query: &SessionQuery)
    -> Result<Vec<SpendLogListRow>, ListStoreError>;
}

pub struct PostgresLogList {
    pub store: PostgresStore,
}

// one positional value's declared type and the value itself, in placeholder order
fn bound(query: &SessionQuery) -> (Vec<Type>, Vec<&(dyn ToSql + Sync)>) {
    let types: Vec<Type> = query.params.iter().map(SessionParam::sql_type).collect();
    let values: Vec<&(dyn ToSql + Sync)> = query.params.iter().map(SessionParam::bind).collect();
    (types, values)
}

// the row Prisma's raw serialiser hands back: every JSONB column as its text, so hydration decodes metadata afterwards
const ROW_TEXT_SQL: &str = concat!(
    "SELECT (to_jsonb(x) || jsonb_build_object(",
    "'metadata', x.metadata::text, 'request_tags', x.request_tags::text))::text ",
    "FROM ({} ) AS x"
);

#[async_trait]
impl LogListStore for PostgresLogList {
    async fn count_matches(&self, query: &SessionQuery) -> Result<i64, ListStoreError> {
        let client = self.store.session().await?;
        let (types, values) = bound(query);
        let statement = client
            .prepare_typed(&query.sql, &types)
            .await
            .map_err(|source| ListStoreError::Database(backend(source)))?;
        let rows = client
            .query(&statement, &values)
            .await
            .map_err(|source| ListStoreError::Database(backend(source)))?;
        let Some(row) = rows.first() else {
            return Ok(0);
        };
        row.try_get(0)
            .map_err(|source| ListStoreError::Database(backend(source)))
    }

    async fn list_rows(
        &self,
        query: &SessionQuery,
    ) -> Result<Vec<SpendLogListRow>, ListStoreError> {
        let client = self.store.session().await?;
        let (types, values) = bound(query);
        let sql = ROW_TEXT_SQL.replace("{}", &query.sql);
        let statement = client
            .prepare_typed(&sql, &types)
            .await
            .map_err(|source| ListStoreError::Database(backend(source)))?;
        let rows = client
            .query(&statement, &values)
            .await
            .map_err(|source| ListStoreError::Database(backend(source)))?;
        let mut decoded = Vec::with_capacity(rows.len());
        for row in &rows {
            let text: String = row
                .try_get(0)
                .map_err(|source| ListStoreError::Database(backend(source)))?;
            decoded.push(serde_json::from_str(&text).map_err(ListStoreError::Row)?);
        }
        Ok(decoded)
    }
}
