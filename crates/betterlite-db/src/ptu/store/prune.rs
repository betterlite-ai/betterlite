use std::collections::BTreeSet;

use chrono::{DateTime, NaiveDate, TimeDelta, Utc};
use deadpool_postgres::GenericClient;

use super::StoreError;
use crate::daily_activity_sql::PTU_SENTINEL_API_KEY;
use crate::postgres_store::errors::backend;

pub const PRUNE_SKEW_GRACE_SECONDS: i64 = 300;
pub const PRUNE_ID_CHUNK_SIZE: usize = 5_000;

const PRUNE_SQL: &str = "DELETE FROM \"LiteLLM_DailyTeamSpend\" \
    WHERE \"date\" = $1 AND \"api_key\" = $2 AND \"updated_at\" < $3 AND \"model\" = ANY($4)";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PruneChunk {
    pub date: String,
    pub cutoff: DateTime<Utc>,
    pub models: Vec<String>,
}

pub fn prune_cutoff(run_started: DateTime<Utc>) -> Result<DateTime<Utc>, StoreError> {
    run_started
        .checked_sub_signed(TimeDelta::seconds(PRUNE_SKEW_GRACE_SECONDS))
        .ok_or(StoreError::GraceUnderflow { run_started })
}

#[must_use]
pub fn prune_chunks(
    date: &str,
    cutoff: DateTime<Utc>,
    scanned_ids: &BTreeSet<String>,
) -> Vec<PruneChunk> {
    let ordered: Vec<String> = scanned_ids.iter().cloned().collect();
    let mut chunks = Vec::new();
    for models in ordered.chunks(PRUNE_ID_CHUNK_SIZE) {
        chunks.push(PruneChunk {
            date: date.to_owned(),
            cutoff,
            models: models.to_vec(),
        });
    }
    chunks
}

pub async fn prune_unrefreshed_rows<C>(
    client: &C,
    day: NaiveDate,
    run_started: DateTime<Utc>,
    scanned_ids: &BTreeSet<String>,
) -> Result<u64, StoreError>
where
    C: GenericClient + Sync,
{
    let cutoff = prune_cutoff(run_started)?;
    let date = day.to_string();
    let mut deleted: u64 = 0;
    for chunk in prune_chunks(&date, cutoff, scanned_ids) {
        let cutoff_naive = chunk.cutoff.naive_utc();
        let rows = client
            .execute(
                PRUNE_SQL,
                &[
                    &chunk.date,
                    &PTU_SENTINEL_API_KEY,
                    &cutoff_naive,
                    &chunk.models,
                ],
            )
            .await
            .map_err(|error| StoreError::Prune(backend(error)))?;
        deleted = deleted.saturating_add(rows);
    }
    if deleted > 0 {
        tracing::info!(
            "PTU rollup for {date}: pruned {deleted} stale sentinel row(s) of {} deployment(s) considered",
            scanned_ids.len(),
        );
    }
    Ok(deleted)
}
