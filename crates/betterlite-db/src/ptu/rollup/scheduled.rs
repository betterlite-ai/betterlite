use chrono::NaiveDate;

use super::RollupError;
use super::alert::OperatorAlert;
use super::backfill::{BackfillRun, run_ptu_flat_cost_backfill};
use super::daily::{DailyRun, run_ptu_flat_cost_rollup};
use crate::database_failures::metrics::WallClock;
use crate::postgres_store::session::PostgresStore;
use crate::ptu::charges::deployment::RouterEntry;
use crate::ptu::charges::records::RollupResult;
use crate::ptu::store::sleeper::Sleeper;
use crate::queues::pod_lock::LockCache;
use crate::queues::pod_lock::PodLockManager;
use crate::queues::pod_lock::owner::lease_key;

pub const PTU_ROLLUP_JOB_ID: &str = "ptu_flat_cost_rollup_job";
pub const PTU_ROLLUP_LOCK_TTL_SECONDS: i64 = 900;
pub const PTU_LAPSED_ALERT_LIMIT: usize = 10;

pub struct ScheduledRun<'run, 'entries, L: LockCache, A: OperatorAlert, S: Sleeper + ?Sized> {
    pub store: &'run PostgresStore,
    pub entries: &'entries [RouterEntry],
    pub clock: &'run dyn WallClock,
    pub sleeper: &'run mut S,
    pub lock: Option<&'run PodLockManager<L>>,
    pub alert: Option<&'run A>,
    pub attribution_enabled: bool,
    pub target_date: Option<NaiveDate>,
}

pub async fn run_scheduled_ptu_rollup<L, A, S>(
    run: ScheduledRun<'_, '_, L, A, S>,
) -> Result<Option<RollupResult>, RollupError>
where
    L: LockCache,
    A: OperatorAlert,
    S: Sleeper + ?Sized,
{
    if !run.attribution_enabled {
        return Ok(None);
    }
    let Some(lock) = run.lock.filter(|manager| manager.cache.is_some()) else {
        return run_and_alert(run, false).await.map(Some);
    };
    let acquired = lock
        .acquire_lock(PTU_ROLLUP_JOB_ID, Some(PTU_ROLLUP_LOCK_TTL_SECONDS), true)
        .await;
    if acquired != Some(true) {
        if lock_is_held(lock).await {
            tracing::info!("PTU rollup: another pod holds the rollup lock, skipping this run");
            return Ok(None);
        }
        tracing::warn!(
            "PTU rollup: could not take the rollup lock and no other pod holds it, running unguarded rather than skipping the day"
        );
        return run_and_alert(run, false).await.map(Some);
    }
    let outcome = run_and_alert(run, true).await;
    lock.release_lock(PTU_ROLLUP_JOB_ID).await;
    outcome.map(Some)
}

// A Redis that cannot be read reports "not held" so the caller runs the day; being wrong here costs a duplicate reconcile.
async fn lock_is_held<L>(lock: &PodLockManager<L>) -> bool
where
    L: LockCache,
{
    let Some(cache) = lock.cache.as_deref() else {
        return false;
    };
    match cache.get_owner(&lease_key(PTU_ROLLUP_JOB_ID)).await {
        Ok(Some(owner)) => !owner.is_empty(),
        Ok(None) => false,
        Err(error) => {
            tracing::warn!("PTU rollup: could not read the rollup lock: {error}");
            false
        }
    }
}

async fn run_and_alert<L, A, S>(
    run: ScheduledRun<'_, '_, L, A, S>,
    may_prune: bool,
) -> Result<RollupResult, RollupError>
where
    L: LockCache,
    A: OperatorAlert,
    S: Sleeper + ?Sized,
{
    let ScheduledRun {
        store,
        entries,
        clock,
        sleeper,
        alert,
        target_date,
        ..
    } = run;
    let result = run_ptu_flat_cost_rollup(DailyRun {
        store,
        entries,
        clock,
        sleeper: &mut *sleeper,
        target_date,
        may_prune,
    })
    .await?;
    if result.rows_failed > 0 {
        let total = result.rows_written.saturating_add(result.rows_failed);
        let day = result.day;
        let failed = result.rows_failed;
        deliver_alert(
            alert,
            &format!(
                "PTU flat-cost rollup for {day}: {failed} of {total} team charges failed to write. Those teams show no PTU cost for that date until the rollup is rerun for it."
            ),
        )
        .await;
    }
    if !result.lapsed.is_empty() {
        deliver_alert(alert, &lapsed_message(&result.lapsed)).await;
    }
    if target_date.is_none() {
        backfill_and_alert(store, entries, clock, sleeper, alert).await;
    }
    Ok(result)
}

async fn backfill_and_alert<A, S>(
    store: &PostgresStore,
    entries: &[RouterEntry],
    clock: &dyn WallClock,
    sleeper: &mut S,
    alert: Option<&A>,
) where
    A: OperatorAlert,
    S: Sleeper + ?Sized,
{
    let outcome = run_ptu_flat_cost_backfill(BackfillRun {
        store,
        entries,
        clock,
        sleeper: &mut *sleeper,
        today: None,
    })
    .await;
    let backfill = match outcome {
        Ok(backfill) => backfill,
        Err(error) => {
            tracing::error!(
                "PTU backfill: catch-up pass failed, the day's rollup still stands: {error}"
            );
            return;
        }
    };
    if backfill.rows_failed > 0 {
        let start = backfill.start;
        let end = backfill.end;
        let failed = backfill.rows_failed;
        let total = backfill.rows_written.saturating_add(backfill.rows_failed);
        deliver_alert(
            alert,
            &format!(
                "PTU flat-cost backfill for {start} to {end}: {failed} of {total} previously unpriced charges failed to write. Those days stay unpriced until a later run picks them up."
            ),
        )
        .await;
    }
}

fn lapsed_message(lapsed: &[String]) -> String {
    let shown = lapsed
        .iter()
        .take(PTU_LAPSED_ALERT_LIMIT)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    let count = lapsed.len();
    format!(
        "PTU flat-cost attribution has stopped for {count} deployment(s) whose effective window has closed: {shown}. Reserved capacity is billed until the deployment is deleted, so a deployment still serving traffic is still being charged for by the provider with nothing attributing it here. Extend the window, or retire the deployment."
    )
}

async fn deliver_alert<A>(alert: Option<&A>, message: &str)
where
    A: OperatorAlert,
{
    let Some(channel) = alert else {
        return;
    };
    if let Err(error) = channel.send(message).await {
        tracing::error!("PTU rollup: could not deliver the failed-charge alert: {error}");
    }
}
