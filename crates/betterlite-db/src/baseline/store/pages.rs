use deadpool_postgres::Transaction;
use tokio_postgres::Row;
use uuid::Uuid;

use super::error::StoreError;
use super::sql::{LOCK_COMPARISON, READ_PAGE};
use crate::baseline::publication::change::change;
use crate::baseline::publication::decode::decode_baseline_publication;
use crate::baseline::publication::model::{BaselinePublication, PublicationStatus};
use crate::baseline::publication::project::StoredRecord;
use crate::baseline::publication::publish::publish;
use crate::baseline::records::decode_baseline_record;

// A page holds this many distinct start times, so a tie never splits at a row-count boundary.
pub const PAGE_TIMESTAMPS: i32 = 128;

// The comparison row as FOR UPDATE returns it.
pub struct Comparison {
    pub revision: i64,
    pub published_revision: i64,
    pub initial_equivalent: bool,
    pub retired: bool,
    pub history: Option<String>,
}

// The filter every page of one replay or withdrawal shares; only the cursor moves between pages.
pub struct PageQuery<'scope> {
    pub scope: &'scope str,
    pub after_revision: i64,
    pub withdraw_from: Option<f64>,
}

pub async fn lock_comparison(
    transaction: &Transaction<'_>,
    scope: &str,
) -> Result<Option<Comparison>, StoreError> {
    let Some(row) = transaction.query_opt(LOCK_COMPARISON, &[&scope]).await? else {
        return Ok(None);
    };
    Ok(Some(Comparison {
        revision: row.try_get("revision")?,
        published_revision: row.try_get("published_revision")?,
        initial_equivalent: row.try_get("initial_equivalent")?,
        retired: row.try_get("retired")?,
        history: row.try_get("history")?,
    }))
}

pub fn stored_record(row: &Row) -> Result<StoredRecord, tokio_postgres::Error> {
    Ok(StoredRecord {
        data: row.try_get("data")?,
        publication: row.try_get("publication")?,
        conflicted: row.try_get("conflicted")?,
        started_at: row.try_get("started_at")?,
    })
}

// _READ_PAGE: every observation of the next distinct start times after the cursor, in started_at, request_id order.
pub async fn read_page(
    transaction: &Transaction<'_>,
    query: &PageQuery<'_>,
    cursor: Option<f64>,
) -> Result<Vec<StoredRecord>, StoreError> {
    let rows = transaction
        .query(
            READ_PAGE,
            &[
                &query.scope,
                &query.after_revision,
                &cursor,
                &PAGE_TIMESTAMPS,
                &query.withdraw_from,
            ],
        )
        .await?;
    Ok(rows.iter().map(stored_record).collect::<Result<_, _>>()?)
}

// The unknown record that replaces a publication, carrying no amounts and no token evidence.
#[must_use]
pub fn unknown_publication(scope: &str, started_at: f64, reason: &str) -> BaselinePublication {
    BaselinePublication {
        version: 3,
        comparison_id: scope.to_owned(),
        comparison_started_at: started_at,
        status: PublicationStatus::Unknown,
        reason: reason.to_owned(),
        provenance: None,
        actual_spend: None,
        baseline_spend: None,
        input_tokens: None,
        cache_read_input_tokens: None,
        cache_creation_5m_input_tokens: None,
        cache_creation_1h_input_tokens: None,
    }
}

// _withdraw: every estimate at or after the new evidence goes back to pending, reversing what it reported.
pub async fn withdraw<G>(
    transaction: &Transaction<'_>,
    scope: &str,
    started_at: f64,
    generate_id: &G,
) -> Result<(), StoreError>
where
    G: Fn() -> Uuid + Sync,
{
    let query = PageQuery {
        scope,
        after_revision: 0,
        withdraw_from: Some(started_at),
    };
    let mut cursor = None;
    loop {
        let page = read_page(transaction, &query, cursor).await?;
        let Some(last) = page.last() else {
            return Ok(());
        };
        cursor = Some(last.started_at);
        let mut changes = Vec::new();
        for row in &page {
            let Some(text) = &row.publication else {
                continue;
            };
            let previous = decode_baseline_publication(text.as_bytes())?;
            let record = decode_baseline_record(row.data.as_bytes())?;
            let start = previous.comparison_started_at.min(started_at);
            let pending = unknown_publication(scope, start, "pending_projection");
            changes.push(change(&record, Some(&previous), &pending));
        }
        publish(transaction, &changes, generate_id).await?;
    }
}
