use uuid::Uuid;

use crate::daily_spend_sql::entities::{DailySpendEntity, SpendRow};
use crate::daily_spend_sql::write::{DailyWriteError, write_bulk_upsert};
use crate::postgres_store::errors::{DbError, backend};
use crate::postgres_store::session::PostgresStore;

use super::super::phase::{CommitPhase, CommitPhaseError, CommitTransition};
use super::super::policy::{SpendFailure, SpendUpdateFailure, classify_spend_failure};
use super::super::transaction::{SpendDeadline, SpendTransactionError};
use super::{ChunkFuture, DailyChunkFailure, DailyChunkWriter};

#[derive(Debug, thiserror::Error)]
pub enum DailyChunkError {
    #[error("daily spend upsert failed")]
    Write(#[from] DailyWriteError),
    #[error("the daily spend commit phase rejected the commit")]
    Phase(#[from] CommitPhaseError),
    #[error("the daily spend transaction timed out")]
    Timeout,
}

impl From<DbError> for DailyChunkError {
    fn from(source: DbError) -> Self {
        Self::Write(DailyWriteError::Statement(source))
    }
}

impl SpendUpdateFailure for DailyChunkError {
    fn spend_failure(&self) -> SpendFailure {
        match self {
            Self::Write(DailyWriteError::Statement(source)) => classify_spend_failure(source),
            Self::Write(DailyWriteError::Input(_)) | Self::Timeout | Self::Phase(_) => {
                SpendFailure::Other
            }
        }
    }
}

impl DailyChunkFailure for DailyChunkError {
    fn is_cancelled(&self) -> bool {
        matches!(self, Self::Phase(CommitPhaseError::Cancelled))
    }
}

pub struct PostgresDailyChunks {
    pub store: PostgresStore,
}

impl PostgresDailyChunks {
    // The source's execute_raw inside _spend_update_tx: the statement applies, the phase turns committing, and only a successful commit clears it again.
    pub(super) async fn commit_transaction(
        &self,
        entity: DailySpendEntity,
        rows: &[SpendRow],
        phase: &CommitPhase,
    ) -> Result<(), DailyChunkError> {
        let mut client = self.store.session().await.map_err(DailyChunkError::from)?;
        let transaction = client
            .transaction()
            .await
            .map_err(|source| DailyChunkError::from(backend(source)))?;
        write_bulk_upsert(&transaction, entity, rows, Uuid::new_v4).await?;
        phase.apply(CommitTransition::Begin)?;
        transaction
            .commit()
            .await
            .map_err(|source| DailyChunkError::from(backend(source)))?;
        phase.apply(CommitTransition::Finish)?;
        Ok(())
    }
}

impl DailyChunkWriter for PostgresDailyChunks {
    type Error = DailyChunkError;

    fn write_chunk<'writer>(
        &'writer self,
        entity: DailySpendEntity,
        rows: &'writer [SpendRow],
        phase: &'writer CommitPhase,
    ) -> ChunkFuture<'writer, DailyChunkError> {
        Box::pin(async move {
            match SpendDeadline::SOURCE
                .run(self.commit_transaction(entity, rows, phase))
                .await
            {
                Ok(outcome) => outcome,
                Err(SpendTransactionError::Work(error)) => Err(error),
                Err(SpendTransactionError::Timeout { .. }) => Err(DailyChunkError::Timeout),
            }
        })
    }
}
