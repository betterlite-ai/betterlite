use async_trait::async_trait;
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::table_ops::rows::{RowError, SpendLogRow};
use serde::de::DeserializeOwned;
use tokio_postgres::types::ToSql;

use super::error::StoreError;
use super::rows::SpendLogRowsQuery;
use crate::spend::log_payload::daily_summary::{DailySummaryRow, SummaryQuery};

#[async_trait]
pub trait LogResultsStore: Send + Sync {
    async fn spend_logs(&self, query: &SpendLogRowsQuery) -> Result<Vec<SpendLogRow>, StoreError>;
    async fn daily_summary(&self, query: &SummaryQuery)
    -> Result<Vec<DailySummaryRow>, StoreError>;
}

pub struct PostgresLogResults {
    pub store: PostgresStore,
}

impl PostgresLogResults {
    // both listings read one text column holding the row as JSON
    async fn decoded<Item>(&self, sql: &str, params: &[String]) -> Result<Vec<Item>, StoreError>
    where
        Item: DeserializeOwned,
    {
        let client = self.store.session().await?;
        let mut bound: Vec<&(dyn ToSql + Sync)> = Vec::with_capacity(params.len());
        for value in params {
            bound.push(value);
        }
        let rows = client
            .query(sql, &bound)
            .await
            .map_err(|source| StoreError::Database(backend(source)))?;
        let mut decoded = Vec::with_capacity(rows.len());
        for row in &rows {
            let text: String = row
                .try_get(0)
                .map_err(|source| StoreError::Database(backend(source)))?;
            decoded.push(
                serde_json::from_str(&text)
                    .map_err(|source| StoreError::Row(RowError::Model(source)))?,
            );
        }
        Ok(decoded)
    }
}

#[async_trait]
impl LogResultsStore for PostgresLogResults {
    async fn spend_logs(&self, query: &SpendLogRowsQuery) -> Result<Vec<SpendLogRow>, StoreError> {
        self.decoded(&query.sql, &query.params).await
    }

    async fn daily_summary(
        &self,
        query: &SummaryQuery,
    ) -> Result<Vec<DailySummaryRow>, StoreError> {
        // the grouped row as one JSON object, the shape decoded reads
        let sql = format!("SELECT row_to_json(x)::text FROM ({}) x", query.sql);
        self.decoded(&sql, &query.params).await
    }
}
