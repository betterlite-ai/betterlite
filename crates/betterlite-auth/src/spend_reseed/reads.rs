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

pub(super) async fn counter_row<Rows>(
    gate: Option<&LoopBoundSemaphore>,
    rows: &Rows,
    lookup: OwnedLookup,
) -> LookupAnswer<SpendRow, Rows::Error>
where
    Rows: SpendCounterRows,
{
    let _permit = match gate.and_then(LoopBoundSemaphore::current) {
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

// the row a decoded counter names, and whether that counter is an end-user one
fn lookup_of(key: SpendCounterKey<'_>) -> Option<(SpendRowLookup<'_>, bool)> {
    match key {
        SpendCounterKey::Key(token) => Some((SpendRowLookup::Key(token), false)),
        SpendCounterKey::TeamMember { user_id, team_id } => {
            Some((SpendRowLookup::TeamMember { user_id, team_id }, false))
        }
        SpendCounterKey::Team(team_id) => Some((SpendRowLookup::Team(team_id), false)),
        SpendCounterKey::User(user_id) => Some((SpendRowLookup::User(user_id), false)),
        SpendCounterKey::Organization(organization_id) => {
            Some((SpendRowLookup::Organization(organization_id), false))
        }
        SpendCounterKey::Project(project_id) => Some((SpendRowLookup::Project(project_id), false)),
        SpendCounterKey::EndUser(user_id) => Some((SpendRowLookup::EndUser(user_id), true)),
        SpendCounterKey::Window { .. } | SpendCounterKey::Unknown(_) => None,
    }
}

// _counter_row's prefix order and _is_key_or_team_window_counter both live in B1735's decoder
pub(super) fn primary_lookup<'key>(
    counter_key: &'key str,
    durations: &dyn DurationParser,
    now: DateTime<Utc>,
) -> Option<SpendRowLookup<'key>> {
    let windows = CounterDurations { durations, now };
    let decoded = decode_spend_counter_key(counter_key.as_bytes(), &windows).ok()?;
    match lookup_of(decoded) {
        Some((lookup, false)) => Some(lookup),
        Some((_, true)) | None => None,
    }
}

// the floor reader accepts only the end-user prefix; every other counter shape reads no row
pub(super) fn end_user_lookup<'key>(
    key: &'key str,
    parser: &dyn DurationParser,
    moment: DateTime<Utc>,
) -> Option<SpendRowLookup<'key>> {
    let decoded = decode_spend_counter_key(
        key.as_bytes(),
        &CounterDurations {
            durations: parser,
            now: moment,
        },
    )
    .ok()?;
    if let SpendCounterKey::EndUser(user_id) = decoded {
        return Some(SpendRowLookup::EndUser(user_id));
    }
    None
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
