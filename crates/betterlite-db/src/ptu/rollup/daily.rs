use chrono::NaiveDate;
use uuid::Uuid;

use super::RollupError;
use super::open;
use super::yesterday;
use crate::database_failures::metrics::WallClock;
use crate::postgres_store::session::PostgresStore;
use crate::ptu::charges::deployment::RouterEntry;
use crate::ptu::charges::pricing::aggregate_charges;
use crate::ptu::charges::records::RollupResult;
use crate::ptu::charges::window::lapsed_models;
use crate::ptu::store::models::load_models;
use crate::ptu::store::prune::prune_unrefreshed_rows;
use crate::ptu::store::sentinel::{ChargeWrite, write_charge_with_retry};
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
    let DailyRun {
        store,
        entries,
        clock,
        sleeper,
        target_date,
        may_prune,
    } = run;
    let day = match target_date {
        Some(day) => day,
        None => yesterday(clock)?,
    };
    let Some(client) = open(store).await? else {
        tracing::warn!("PTU rollup: no database is connected, skipping");
        return Ok(RollupResult {
            day,
            models_processed: 0,
            rows_written: 0,
            rows_failed: 0,
            lapsed: Vec::new(),
        });
    };
    let run_started = clock.now();
    let loaded = load_models(&client, entries).await?;
    let charges = aggregate_charges(&loaded.models, day)?;
    let mut rows_written = 0_usize;
    for charge in &charges {
        let write = ChargeWrite {
            charge,
            date: day,
            id: Uuid::new_v4(),
            updated_at: clock.now(),
        };
        if write_charge_with_retry(&client, &mut *sleeper, &write).await {
            rows_written = rows_written.saturating_add(1);
        }
    }
    let rows_failed = charges.len().saturating_sub(rows_written);
    if !may_prune {
        tracing::info!(
            "PTU rollup for {day}: ran without the cross-pod lock, skipping the prune so a concurrent pod's charges cannot be swept by this run's cutoff"
        );
    } else if rows_failed > 0 {
        tracing::warn!(
            "PTU rollup: {rows_failed} charge(s) failed for {day}, skipping the prune so a row whose replacement did not land is not deleted; rerun that date to reconcile"
        );
    } else {
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
