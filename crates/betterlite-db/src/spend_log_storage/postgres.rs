use std::slice::from_ref;

use deadpool_postgres::GenericClient;

use crate::daily_spend_sql::bind::{SqlBind, execute};
use crate::database_faults::native::{
    NativeFault, classify_sqlstate, is_native_transport_error, native_sqlstate,
};
use crate::postgres_store::errors::{DbError, backend};
use crate::postgres_store::session::PostgresStore;
use crate::redis_spend::log_row::SpendLogRow;

use super::batch_claim::{BatchChargeRow, BatchClaimTable};
use super::sql::{SELECT_BATCH_ROW, insert_statement, takeover_statement};
use super::{SpendLogFuture, SpendLogWriteError, SpendLogsWriter};

// A spend log statement either reached the database or never left the process: only the first half carries a retry or a poison decision.
#[derive(Debug, thiserror::Error)]
pub enum SpendLogStoreError {
    #[error("the spend log statement failed")]
    Database(#[source] DbError),
    #[error("the spend log rows could not be encoded")]
    Json(#[source] serde_json::Error),
}

impl SpendLogStoreError {
    const fn database(&self) -> Option<&DbError> {
        match self {
            Self::Database(error) => Some(error),
            Self::Json(_) => None,
        }
    }
}

// is_prisma_data_error: Postgres read the statement and refused the data itself, so one row may be poison.
// SQLSTATE class 22 is that rejection; a unique violation, a foreign key or a missing required value is its own prisma error and keeps its own meaning.
fn native_data_rejection<E>(error: &DbError<E>) -> bool {
    match error {
        DbError::SqlState { code, .. } => code.code().starts_with("22"),
        DbError::NoDbConnected
        | DbError::Pool { .. }
        | DbError::Backend { .. }
        | DbError::InvalidRecord { .. }
        | DbError::CreateNull
        | DbError::UniqueViolation { .. } => false,
    }
}

fn native_deadlock<E>(error: &DbError<E>) -> bool {
    native_sqlstate(error).is_some_and(|code| classify_sqlstate(&code) == NativeFault::Deadlock)
}

impl SpendLogWriteError for SpendLogStoreError {
    fn is_transient(&self) -> bool {
        self.database()
            .is_some_and(|error| native_deadlock(error) || is_native_transport_error(error))
    }

    fn is_data_rejection(&self) -> bool {
        self.database().is_some_and(native_data_rejection)
    }

    fn is_service_unavailable(&self) -> bool {
        self.database().is_some_and(is_native_transport_error)
    }

    fn is_deadlock(&self) -> bool {
        self.database().is_some_and(native_deadlock)
    }

    fn is_database(&self) -> bool {
        matches!(self, Self::Database(_))
    }
}

// SpendLogsRepository over the pinned LiteLLM_SpendLogs table.
pub struct PostgresSpendLogsWriter {
    pub store: PostgresStore,
}

impl SpendLogsWriter for PostgresSpendLogsWriter {
    type Error = SpendLogStoreError;

    fn insert_rows<'call>(
        &'call self,
        rows: &'call [SpendLogRow],
    ) -> SpendLogFuture<'call, Result<usize, SpendLogStoreError>> {
        Box::pin(async move { insert_spend_log_rows(&self.store, rows).await })
    }
}

impl BatchClaimTable for PostgresSpendLogsWriter {
    fn find_batch_row<'call>(
        &'call self,
        request_id: &'call str,
    ) -> SpendLogFuture<'call, Result<Option<BatchChargeRow>, SpendLogStoreError>> {
        Box::pin(async move {
            let client = self
                .store
                .session()
                .await
                .map_err(SpendLogStoreError::Database)?;
            find_batch_row_with_client(&client, request_id).await
        })
    }

    fn take_over<'call>(
        &'call self,
        data: &'call SpendLogRow,
        request_id: &'call str,
    ) -> SpendLogFuture<'call, Result<usize, SpendLogStoreError>> {
        Box::pin(async move {
            let client = self
                .store
                .session()
                .await
                .map_err(SpendLogStoreError::Database)?;
            let payload = encode_rows(from_ref(data))?;
            let statement = takeover_statement(data);
            let params = [
                SqlBind::Text(Some(payload)),
                SqlBind::Text(Some(request_id.to_owned())),
            ];
            let updated = execute(&client, &statement, &params)
                .await
                .map_err(|driver| SpendLogStoreError::Database(backend(driver)))?;
            Ok(usize::try_from(updated).unwrap_or(usize::MAX))
        })
    }
}

// create_many(data=[...], skip_duplicates=True): one statement inserts the whole batch and reports how many rows were new.
pub async fn insert_spend_log_rows(
    store: &PostgresStore,
    rows: &[SpendLogRow],
) -> Result<usize, SpendLogStoreError> {
    if rows.is_empty() {
        return Ok(0);
    }
    let client = store
        .session()
        .await
        .map_err(SpendLogStoreError::Database)?;
    let payload = encode_rows(rows)?;
    let inserted = execute(
        &client,
        &insert_statement(),
        &[SqlBind::Text(Some(payload))],
    )
    .await
    .map_err(|driver| SpendLogStoreError::Database(backend(driver)))?;
    Ok(usize::try_from(inserted).unwrap_or(usize::MAX))
}

// find_unique(where={"request_id": ...}) after the insert skipped a row the batch had already written.
async fn find_batch_row_with_client<C>(
    client: &C,
    request_id: &str,
) -> Result<Option<BatchChargeRow>, SpendLogStoreError>
where
    C: GenericClient + Sync,
{
    let row = client
        .query_opt(SELECT_BATCH_ROW, &[&request_id])
        .await
        .map_err(|driver| SpendLogStoreError::Database(backend(driver)))?;
    Ok(row.map(|found| BatchChargeRow {
        call_type: found.get("call_type"),
        status: found.get("status"),
        spend: found.get("spend"),
    }))
}

fn encode_rows(rows: &[SpendLogRow]) -> Result<String, SpendLogStoreError> {
    serde_json::to_string(rows).map_err(SpendLogStoreError::Json)
}
