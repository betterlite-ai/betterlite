use std::convert::Infallible;

use chrono::NaiveDateTime;
use deadpool_postgres::PoolError;
use tokio_postgres::types::ToSql;

use crate::postgres_store::errors::{DbError, backend};
use crate::postgres_store::session::PostgresStore;

use super::index_rows::ToolIndexRow;
use super::rollups::ToolDayRollup;
use super::{FlushFuture, ToolSpendFailure, ToolSpendWriter};

const INSERT_INDEX_SQL: &str =
    r#"INSERT INTO "LiteLLM_SpendLogToolIndex" (request_id, tool_name, start_time) VALUES "#;
const SKIP_DUPLICATE_INDEX_SQL: &str = r#" ON CONFLICT (request_id, tool_name) DO NOTHING"#;
const UPSERT_ROLLUP_SQL: &str = r#"INSERT INTO "LiteLLM_DailyToolSpend" (date, tool_name, spend, total_tokens, request_count, updated_at) VALUES ($1, $2, $3::numeric::double precision, $4, $5, now()) ON CONFLICT (date, tool_name) DO UPDATE SET spend = "LiteLLM_DailyToolSpend".spend + EXCLUDED.spend, total_tokens = "LiteLLM_DailyToolSpend".total_tokens + EXCLUDED.total_tokens, request_count = "LiteLLM_DailyToolSpend".request_count + EXCLUDED.request_count, updated_at = now()"#;

pub struct PostgresToolSpendWriter {
    pub store: PostgresStore,
}

#[derive(Debug, thiserror::Error)]
pub enum ToolSpendWriteError {
    #[error("the tool spend flush could not reach the database")]
    Connect(#[source] DbError),
    #[error("the tool spend flush failed after its statements were sent")]
    AfterSend(#[source] DbError),
}

// Only a pool that could not open a connection proves the statements never reached the database
impl ToolSpendFailure for ToolSpendWriteError {
    fn is_pre_send(&self) -> bool {
        match self {
            Self::Connect(DbError::Pool { source }) => {
                matches!(&source.driver, PoolError::Backend(_))
            }
            Self::Connect(_) | Self::AfterSend(_) => false,
        }
    }
}

impl From<tokio_postgres::Error> for ToolSpendWriteError {
    fn from(driver: tokio_postgres::Error) -> Self {
        Self::AfterSend(backend::<Infallible>(driver))
    }
}

impl ToolSpendWriter for PostgresToolSpendWriter {
    type Error = ToolSpendWriteError;

    fn write_index_rows<'writer>(
        &'writer self,
        rows: &'writer [ToolIndexRow],
    ) -> FlushFuture<'writer, Self::Error> {
        Box::pin(async move {
            if rows.is_empty() {
                return Ok(());
            }
            let client = self
                .store
                .session()
                .await
                .map_err(ToolSpendWriteError::Connect)?;
            let times: Vec<NaiveDateTime> =
                rows.iter().map(|row| row.start_time.naive_utc()).collect();
            let mut params: Vec<&(dyn ToSql + Sync)> =
                Vec::with_capacity(rows.len().saturating_mul(3));
            for (row, time) in rows.iter().zip(&times) {
                params.push(&row.request_id);
                params.push(&row.tool_name);
                params.push(time);
            }
            let statement = index_statement(rows.len());
            client
                .execute(&statement, &params)
                .await
                .map_err(ToolSpendWriteError::from)?;
            Ok(())
        })
    }

    fn apply_rollups<'writer>(
        &'writer self,
        rollups: &'writer [ToolDayRollup],
    ) -> FlushFuture<'writer, Self::Error> {
        Box::pin(async move {
            if rollups.is_empty() {
                return Ok(());
            }
            let mut client = self
                .store
                .session()
                .await
                .map_err(ToolSpendWriteError::Connect)?;
            let transaction = client
                .transaction()
                .await
                .map_err(ToolSpendWriteError::from)?;
            for rollup in rollups {
                transaction
                    .execute(
                        UPSERT_ROLLUP_SQL,
                        &[
                            &rollup.date,
                            &rollup.tool_name,
                            &rollup.spend,
                            &rollup.total_tokens,
                            &rollup.request_count,
                        ],
                    )
                    .await
                    .map_err(ToolSpendWriteError::from)?;
            }
            transaction
                .commit()
                .await
                .map_err(ToolSpendWriteError::from)
        })
    }
}

// One statement carries the whole slice, so its payload stays inside the statement budget
fn index_statement(row_count: usize) -> String {
    let mut groups = Vec::with_capacity(row_count);
    for index in 0..row_count {
        let base = index.saturating_mul(3);
        groups.push(format!(
            "(${}, ${}, ${})",
            base.saturating_add(1),
            base.saturating_add(2),
            base.saturating_add(3)
        ));
    }
    format!(
        "{INSERT_INDEX_SQL}{}{SKIP_DUPLICATE_INDEX_SQL}",
        groups.join(", ")
    )
}
