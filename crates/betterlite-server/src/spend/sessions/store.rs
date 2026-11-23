use async_trait::async_trait;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::table_ops::rows::RowError;
use serde::de::DeserializeOwned;
use tokio_postgres::types::{ToSql, Type};

use crate::spend::log_results::projection::SpendLogListRow;

use super::params::{SessionParam, SessionQuery};
use super::ports::{SessionGroupCount, SessionGroupRow, SessionsStore, SessionsStoreError};
use super::rows::SessionSpendRow;

pub struct PostgresSessions {
    pub store: PostgresStore,
}

impl PostgresSessions {
    // every session read returns one text column holding its row as JSON
    async fn decoded<Item>(&self, query: &SessionQuery) -> Result<Vec<Item>, SessionsStoreError>
    where
        Item: DeserializeOwned,
    {
        let client = self.store.session().await?;
        let types: Vec<Type> = query.params.iter().map(SessionParam::sql_type).collect();
        let bound: Vec<&(dyn ToSql + Sync)> = query.params.iter().map(SessionParam::bind).collect();
        let sql = format!("SELECT row_to_json(x)::text FROM ({}) x", query.sql);
        let statement = client
            .prepare_typed(&sql, &types)
            .await
            .map_err(|source| SessionsStoreError::Database(backend(source)))?;
        let rows = client
            .query(&statement, &bound)
            .await
            .map_err(|source| SessionsStoreError::Database(backend(source)))?;
        let mut decoded = Vec::with_capacity(rows.len());
        for row in &rows {
            let text: String = row
                .try_get(0)
                .map_err(|source| SessionsStoreError::Database(backend(source)))?;
            decoded.push(
                serde_json::from_str(&text)
                    .map_err(|source| SessionsStoreError::Row(RowError::Model(source)))?,
            );
        }
        Ok(decoded)
    }

    async fn counted(&self, query: &SessionQuery) -> Result<i64, SessionsStoreError> {
        match self
            .decoded::<SessionGroupCount>(query)
            .await?
            .into_iter()
            .next()
        {
            Some(row) => Ok(row.total_count),
            None => Ok(0),
        }
    }
}

// the Prisma query_raw serialiser hands a JSONB column back as its text, so hydration decodes metadata afterwards while request_tags stays the text it reads
fn prisma_text(value: Option<JsonData>) -> Result<Option<JsonData>, SessionsStoreError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let text = serde_json::to_string(&value)
        .map_err(|source| SessionsStoreError::Row(RowError::Json(source)))?;
    Ok(Some(JsonData::String(text)))
}

#[async_trait]
impl SessionsStore for PostgresSessions {
    async fn session_groups(
        &self,
        query: &SessionQuery,
    ) -> Result<Vec<SessionGroupRow>, SessionsStoreError> {
        self.decoded(query).await
    }

    async fn session_group_count(&self, query: &SessionQuery) -> Result<i64, SessionsStoreError> {
        self.counted(query).await
    }

    async fn session_representatives(
        &self,
        query: &SessionQuery,
    ) -> Result<Vec<SpendLogListRow>, SessionsStoreError> {
        let mut rows: Vec<SpendLogListRow> = self.decoded(query).await?;
        for row in &mut rows {
            row.metadata = prisma_text(row.metadata.take())?;
            row.request_tags = prisma_text(row.request_tags.take())?;
        }
        Ok(rows)
    }

    async fn session_rows(
        &self,
        query: &SessionQuery,
    ) -> Result<Vec<SessionSpendRow>, SessionsStoreError> {
        self.decoded(query).await
    }

    async fn session_row_count(&self, query: &SessionQuery) -> Result<i64, SessionsStoreError> {
        self.counted(query).await
    }
}
