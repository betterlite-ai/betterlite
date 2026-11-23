use deadpool_postgres::Transaction;
use tokio::time::timeout;
use uuid::Uuid;

use super::error::StoreError;
use super::pages::{lock_comparison, stored_record, unknown_publication, withdraw};
use super::sql::{
    ADVANCE_REVISION, CREATE_COMPARISON, INSERT_RECORD, MARK_CONFLICT, READ_CANONICAL,
};
use super::{ACCOUNTING_LIMITS, AppendResult, BaselineAccountingStore, TRANSACTION_TIMEOUT, begin};
use crate::autorouter_turn_sql::write::write_autorouter_turn;
use crate::baseline::publication::change::change;
use crate::baseline::publication::decode::decode_baseline_publication;
use crate::baseline::publication::project::StoredRecord;
use crate::baseline::publication::publish::publish;
use crate::baseline::records::canonical::serialize_record;
use crate::baseline::records::{BaselineAccountingRecord, decode_baseline_record};
use crate::database_replica::pinned::writer_wrapper;

// The row that already owns this request ID in the scope, and whether this append marked it conflicted.
struct Existing {
    row: StoredRecord,
    conflicted: bool,
}

// What the insert by globally unique request ID found.
enum Insertion {
    Fresh,
    Duplicate(Existing),
    Foreign,
}

// A failed append only loses the estimate, never the request: the caller keeps the record for a retry.
pub async fn append_record<C, G>(
    store: &BaselineAccountingStore<'_, C, G>,
    record: &BaselineAccountingRecord,
) -> AppendResult
where
    C: Sync,
    G: Fn() -> Uuid + Sync,
{
    let appended = async {
        let mut session = writer_wrapper(store.database).session().await?;
        let transaction = begin(&mut session, ACCOUNTING_LIMITS).await?;
        let outcome = append_rows(&transaction, record, &store.generate_id).await?;
        transaction.commit().await?;
        Ok::<_, StoreError>(outcome)
    };
    let settled = timeout(TRANSACTION_TIMEOUT, appended)
        .await
        .unwrap_or_else(|elapsed| Err(StoreError::from(elapsed)));
    settled.unwrap_or_else(|error| {
        tracing::warn!(%error, "Auto-router baseline observation could not be persisted");
        AppendResult::Unavailable
    })
}

async fn append_rows<G>(
    transaction: &Transaction<'_>,
    record: &BaselineAccountingRecord,
    generate_id: &G,
) -> Result<AppendResult, StoreError>
where
    G: Fn() -> Uuid + Sync,
{
    let scope = &record.scope;
    transaction
        .execute(
            CREATE_COMPARISON,
            &[
                scope,
                &record.api_key,
                &record.session_id,
                &record.router_name,
            ],
        )
        .await?;
    let Some(comparison) = lock_comparison(transaction, scope).await? else {
        return Ok(AppendResult::Unavailable);
    };
    let revision = comparison
        .revision
        .checked_add(1)
        .ok_or(StoreError::Revision)?;
    let existing = match insert_observation(transaction, record, revision).await? {
        Insertion::Fresh => None,
        Insertion::Duplicate(found) => Some(found),
        Insertion::Foreign => return Ok(AppendResult::Conflict),
    };
    if comparison.retired {
        publish_retired(transaction, record, existing.as_ref(), generate_id).await?;
        return Ok(AppendResult::Retired);
    }
    if existing.as_ref().is_none_or(|found| found.conflicted) {
        let started_at = existing
            .as_ref()
            .map_or(record.observation.started_at, |found| found.row.started_at);
        withdraw(transaction, scope, started_at, generate_id).await?;
        transaction
            .execute(ADVANCE_REVISION, &[scope, &revision])
            .await?;
    }
    Ok(AppendResult::Recorded)
}

async fn insert_observation(
    transaction: &Transaction<'_>,
    record: &BaselineAccountingRecord,
    revision: i64,
) -> Result<Insertion, StoreError> {
    let observation = &record.observation;
    let request_id = &observation.request_id;
    let scope = &record.scope;
    let data = serialize_record(record);
    let inserted = transaction
        .execute(
            INSERT_RECORD,
            &[request_id, scope, &observation.started_at, &revision, &data],
        )
        .await?;
    if inserted > 0 {
        if let Some(turn) = &record.turn {
            write_autorouter_turn(transaction, turn).await?;
        }
        return Ok(Insertion::Fresh);
    }
    let conflicted = transaction
        .execute(MARK_CONFLICT, &[request_id, scope, &data, &revision])
        .await?
        > 0;
    let canonical = transaction
        .query_opt(READ_CANONICAL, &[request_id, scope])
        .await?;
    let Some(row) = canonical else {
        return Ok(Insertion::Foreign);
    };
    Ok(Insertion::Duplicate(Existing {
        row: stored_record(&row)?,
        conflicted,
    }))
}

// A retired comparison reports the request as unknown, against the canonical row when one already exists.
async fn publish_retired<G>(
    transaction: &Transaction<'_>,
    record: &BaselineAccountingRecord,
    existing: Option<&Existing>,
    generate_id: &G,
) -> Result<(), StoreError>
where
    G: Fn() -> Uuid + Sync,
{
    let canonical = existing.map(|found| &found.row);
    let owner = canonical
        .map(|row| decode_baseline_record(row.data.as_bytes()))
        .transpose()?;
    let previous = canonical
        .and_then(|row| row.publication.as_deref())
        .map(|text| decode_baseline_publication(text.as_bytes()))
        .transpose()?;
    let started_at = canonical.map_or(record.observation.started_at, |row| row.started_at);
    let retired = unknown_publication(&record.scope, started_at, "comparison_retired");
    let correction = change(
        owner.as_ref().unwrap_or(record),
        previous.as_ref(),
        &retired,
    );
    publish(transaction, &[correction], generate_id).await?;
    Ok(())
}
