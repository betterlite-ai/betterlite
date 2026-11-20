use std::error::Error;
use std::future::Future;

use betterlite_db::database_failures::deadline::{DbLookupError, LookupRequest, bounded_db_lookup};
use betterlite_db::database_failures::gate::LoopBoundSemaphore;
use betterlite_db::spend_rows::key::{
    SpendCounterKey, WindowDurationValidity, decode_spend_counter_key,
};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use tokio_util::task::TaskTracker;

use crate::budget_primitives::window_start::DurationParser;
use crate::spend_reseed::cache::ReseedCache;

use super::reseed::{CoalescedError, CoalescedError::Cache as cache_failure};
use super::rows::{LookupAnswer, OwnedLookup, SpendCounterRows, SpendRow, SpendRowLookup};
use super::values::cache_amount;

pub(super) async fn gated_primary<Rows>(
    gate: &LoopBoundSemaphore,
    rows: &Rows,
    lookup: OwnedLookup,
) -> LookupAnswer<SpendRow, Rows::Error>
where
    Rows: SpendCounterRows,
{
    let _permit = match gate.current() {
        Some(semaphore) => semaphore.acquire_owned().await.ok(),
        None => None,
    };
    match rows.spend_row(lookup.borrowed()).await {
        Ok(row) => LookupAnswer::Row(row),
        Err(error) => LookupAnswer::Failed(error),
    }
}

// B199's duration parser decides which suffixes make a key/team counter a window counter
struct CounterDurations<'durations> {
    durations: &'durations (dyn DurationParser + Send + Sync),
    now: DateTime<Utc>,
}

impl WindowDurationValidity for CounterDurations<'_> {
    fn is_valid(&self, spec: &str) -> bool {
        self.durations.seconds(spec, self.now).is_some()
    }
}

// _counter_row's prefix order and _is_key_or_team_window_counter both live in B1735's decoder
pub(super) fn primary_lookup<'key>(
    counter_key: &'key str,
    durations: &dyn DurationParser,
    now: DateTime<Utc>,
) -> Option<SpendRowLookup<'key>> {
    let windows = CounterDurations { durations, now };
    match decode_spend_counter_key(counter_key.as_bytes(), &windows).ok()? {
        SpendCounterKey::Key(token) => Some(SpendRowLookup::Key(token)),
        SpendCounterKey::TeamMember { user_id, team_id } => {
            Some(SpendRowLookup::TeamMember { user_id, team_id })
        }
        SpendCounterKey::Team(team_id) => Some(SpendRowLookup::Team(team_id)),
        SpendCounterKey::User(user_id) => Some(SpendRowLookup::User(user_id)),
        SpendCounterKey::Organization(organization_id) => {
            Some(SpendRowLookup::Organization(organization_id))
        }
        SpendCounterKey::Project(project_id) => Some(SpendRowLookup::Project(project_id)),
        SpendCounterKey::EndUser(_)
        | SpendCounterKey::Window { .. }
        | SpendCounterKey::Unknown(_) => None,
    }
}

// the floor reader accepts only the end-user prefix; every other counter shape reads no row
pub(super) fn end_user_lookup<'key>(
    counter_key: &'key str,
    durations: &dyn DurationParser,
    now: DateTime<Utc>,
) -> Option<SpendRowLookup<'key>> {
    let windows = CounterDurations { durations, now };
    match decode_spend_counter_key(counter_key.as_bytes(), &windows).ok()? {
        SpendCounterKey::EndUser(user_id) => Some(SpendRowLookup::EndUser(user_id)),
        SpendCounterKey::Key(_)
        | SpendCounterKey::TeamMember { .. }
        | SpendCounterKey::Team(_)
        | SpendCounterKey::User(_)
        | SpendCounterKey::Organization(_)
        | SpendCounterKey::Project(_)
        | SpendCounterKey::Window { .. }
        | SpendCounterKey::Unknown(_) => None,
    }
}

pub(super) async fn bounded_row<Failure, Lookup>(
    lookup: Lookup,
    request: LookupRequest<'_>,
    tasks: &TaskTracker,
    counter_key: &str,
) -> Option<Option<SpendRow>>
where
    Failure: Error + Send + Sync + 'static,
    Lookup:
        Future<Output = Result<LookupAnswer<SpendRow, Failure>, DbLookupError>> + Send + 'static,
{
    match bounded_db_lookup(lookup, request, tasks).await {
        Ok(LookupAnswer::Row(row)) => Some(row),
        Ok(LookupAnswer::Failed(error)) => {
            tracing::error!(%error, counter_key, "SpendCounterReseed: the counter row read failed");
            None
        }
        Err(error) => {
            tracing::error!(%error, counter_key, "SpendCounterReseed: the bounded lookup failed");
            None
        }
    }
}

// the re-check after the lock: a batch answer is an authoritative Redis miss, so local memory is stale
pub(super) async fn cached_amount<Cache>(
    cache: &Cache,
    batch_miss: bool,
    counter_key: &str,
) -> Result<Option<Decimal>, CoalescedError<Cache::Error>>
where
    Cache: ReseedCache + ?Sized,
{
    let mut clean_miss = batch_miss;
    if cache.redis_configured() && !clean_miss {
        match cache.redis_get(counter_key).await {
            Ok(Some(value)) => {
                if let Ok(amount) = cache_amount(&value) {
                    return Ok(Some(amount));
                }
            }
            Ok(None) => clean_miss = true,
            // a failed Redis read silently leaves the per-pod cache as the only fallback
            Err(_error) => {}
        }
    }
    if !clean_miss
        && let Some(value) = cache.memory_get(counter_key).await.map_err(cache_failure)?
    {
        return Ok(Some(cache_amount(&value)?));
    }
    Ok(None)
}
