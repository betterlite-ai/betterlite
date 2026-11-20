use std::convert::Infallible;

use deadpool_postgres::PoolError;

use super::{ModelUsageFailure, ModelUsageFuture, ModelUsageRollup, ModelUsageWriter};
use crate::postgres_store::errors::{DbError, backend};
use crate::postgres_store::session::PostgresStore;

const UPSERT_SQL: &str = r#"INSERT INTO "LiteLLM_DailyModelUsage" (date, model_group, model, custom_llm_provider, task_type, spend, prompt_tokens, completion_tokens, request_count, successful_requests, failed_requests, updated_at) VALUES ($1, $2, $3, $4, $5, $6::numeric::double precision, $7, $8, $9, $10, $11, now()) ON CONFLICT (date, model_group, model, custom_llm_provider, task_type) DO UPDATE SET spend = "LiteLLM_DailyModelUsage".spend + EXCLUDED.spend, prompt_tokens = "LiteLLM_DailyModelUsage".prompt_tokens + EXCLUDED.prompt_tokens, completion_tokens = "LiteLLM_DailyModelUsage".completion_tokens + EXCLUDED.completion_tokens, request_count = "LiteLLM_DailyModelUsage".request_count + EXCLUDED.request_count, successful_requests = "LiteLLM_DailyModelUsage".successful_requests + EXCLUDED.successful_requests, failed_requests = "LiteLLM_DailyModelUsage".failed_requests + EXCLUDED.failed_requests, updated_at = now()"#;

pub struct PostgresModelUsageWriter {
    pub store: PostgresStore,
}

#[derive(Debug, thiserror::Error)]
pub enum ModelUsageWriteError {
    #[error("the model usage flush could not reach the database")]
    Connect(#[source] DbError),
    #[error("the model usage flush failed after its statements were sent")]
    AfterSend(#[source] DbError),
}

// Only a pool that could not open a connection proves the statements never reached the database
impl ModelUsageFailure for ModelUsageWriteError {
    fn is_pre_send(&self) -> bool {
        match self {
            Self::Connect(DbError::Pool { source }) => {
                matches!(&source.driver, PoolError::Backend(_))
            }
            Self::Connect(_) | Self::AfterSend(_) => false,
        }
    }
}

impl From<tokio_postgres::Error> for ModelUsageWriteError {
    fn from(driver: tokio_postgres::Error) -> Self {
        Self::AfterSend(backend::<Infallible>(driver))
    }
}

impl PostgresModelUsageWriter {
    // the whole batch shares one transaction, so a failed statement leaves no increment of the flush behind
    async fn write_rollups(
        &self,
        rollups: &[ModelUsageRollup],
    ) -> Result<(), ModelUsageWriteError> {
        if rollups.is_empty() {
            return Ok(());
        }
        let mut client = self
            .store
            .session()
            .await
            .map_err(ModelUsageWriteError::Connect)?;
        let transaction = client
            .transaction()
            .await
            .map_err(ModelUsageWriteError::from)?;
        for rollup in rollups {
            transaction
                .execute(
                    UPSERT_SQL,
                    &[
                        &rollup.key.date,
                        &rollup.key.model_group,
                        &rollup.key.model,
                        &rollup.key.custom_llm_provider,
                        &rollup.key.task_type,
                        &rollup.spend,
                        &rollup.prompt_tokens,
                        &rollup.completion_tokens,
                        &rollup.request_count,
                        &rollup.successful_requests,
                        &rollup.failed_requests,
                    ],
                )
                .await
                .map_err(ModelUsageWriteError::from)?;
        }
        transaction
            .commit()
            .await
            .map_err(ModelUsageWriteError::from)
    }
}

impl ModelUsageWriter for PostgresModelUsageWriter {
    type Error = ModelUsageWriteError;

    fn apply_rollups<'writer>(
        &'writer self,
        rollups: &'writer [ModelUsageRollup],
    ) -> ModelUsageFuture<'writer, Self::Error> {
        Box::pin(self.write_rollups(rollups))
    }
}
