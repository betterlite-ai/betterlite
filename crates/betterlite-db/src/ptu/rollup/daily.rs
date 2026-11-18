use chrono::NaiveDate;
use deadpool_postgres::Client;

use super::RollupError;
use super::open;
use super::write_one;
use super::yesterday;
use crate::database_failures::metrics::WallClock;
use crate::postgres_store::session::PostgresStore;
use crate::ptu::charges::deployment::RouterEntry;
use crate::ptu::charges::pricing::{PtuCharge, aggregate_charges};
use crate::ptu::charges::records::RollupResult;
use crate::ptu::charges::window::lapsed_models;
use crate::ptu::store::models::load_models;
use crate::ptu::store::prune::prune_unrefreshed_rows;
use crate::ptu::store::sleeper::Sleeper;

pub struct DailyRun<'run, 'entries, S: Sleeper + ?Sized> {
    pub store: &'run PostgresStore,
    pub entries: &'entries [RouterEntry],
    pub clock: &'run dyn WallClock,
    pub sleeper: &'run mut S,
    pub target_date: Option<NaiveDate>,
    pub may_prune: bool,
}

pub async fn run_ptu_flat_cost_rollup<S>(
    run: DailyRun<'_, '_, S>,
) -> Result<RollupResult, RollupError>
where
    S: Sleeper + ?Sized,
{
    let day = match run.target_date {
        Some(day) => day,
        None => yesterday(run.clock)?,
    };
    let Some(client) = open(run.store).await? else {
        tracing::warn!("PTU rollup: no database is connected, skipping");
        return Ok(empty_day(day));
    };
    let run_started = run.clock.now();
    let loaded = load_models(&client, run.entries).await?;
    let charges = aggregate_charges(&loaded.models, day)?;
    let rows_written = write_charges(&client, &charges, day, run.clock, run.sleeper).await;
    let rows_failed = charges.len().saturating_sub(rows_written);
    if prune_allowed(run.may_prune, rows_failed, day) {
        prune_unrefreshed_rows(&client, day, run_started, &loaded.scanned_ids).await?;
    }
    let models_processed = loaded.models.len();
    tracing::info!(
        "PTU rollup for {day}: {models_processed} PTU models processed, {rows_written} rows written, {rows_failed} rows failed"
    );
    Ok(RollupResult {
        day,
        models_processed,
        rows_written,
        rows_failed,
        lapsed: lapsed_models(&loaded.models, run_started),
    })
}

const fn empty_day(day: NaiveDate) -> RollupResult {
    RollupResult {
        day,
        models_processed: 0,
        rows_written: 0,
        rows_failed: 0,
        lapsed: Vec::new(),
    }
}

async fn write_charges<S>(
    client: &Client,
    charges: &[PtuCharge],
    day: NaiveDate,
    clock: &dyn WallClock,
    sleeper: &mut S,
) -> usize
where
    S: Sleeper + ?Sized,
{
    let mut rows_written = 0_usize;
    for charge in charges {
        if write_one(client, charge, day, clock, sleeper).await {
            rows_written = rows_written.saturating_add(1);
        }
    }
    rows_written
}

// The source prunes only a lock-guarded run whose charges all landed, so a row whose replacement did not land is never deleted.
fn prune_allowed(may_prune: bool, rows_failed: usize, day: NaiveDate) -> bool {
    if !may_prune {
        tracing::info!(
            "PTU rollup for {day}: ran without the cross-pod lock, skipping the prune so a concurrent pod's charges cannot be swept by this run's cutoff"
        );
        return false;
    }
    if rows_failed > 0 {
        tracing::warn!(
            "PTU rollup: {rows_failed} charge(s) failed for {day}, skipping the prune so a row whose replacement did not land is not deleted; rerun that date to reconcile"
        );
        return false;
    }
    true
}
