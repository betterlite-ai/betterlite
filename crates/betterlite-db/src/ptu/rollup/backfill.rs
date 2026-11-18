use std::collections::BTreeSet;

use chrono::{Days, NaiveDate};
use deadpool_postgres::Client;

use super::RollupError;
use super::open;
use super::write_one;
use super::yesterday;
use crate::database_failures::metrics::WallClock;
use crate::postgres_store::session::PostgresStore;
use crate::ptu::charges::deployment::RouterEntry;
use crate::ptu::charges::pricing::{ChargeError, PtuCharge, PtuModel, aggregate_charges};
use crate::ptu::charges::records::BackfillResult;
use crate::ptu::charges::window::backfill_window;
use crate::ptu::store::models::load_models;
use crate::ptu::store::sentinel::{SentinelKey, existing_sentinel_keys};
use crate::ptu::store::sleeper::Sleeper;

pub struct BackfillRun<'run, 'entries, S: Sleeper + ?Sized> {
    pub store: &'run PostgresStore,
    pub entries: &'entries [RouterEntry],
    pub clock: &'run dyn WallClock,
    pub sleeper: &'run mut S,
    pub today: Option<NaiveDate>,
}

pub async fn run_ptu_flat_cost_backfill<S>(
    run: BackfillRun<'_, '_, S>,
) -> Result<BackfillResult, RollupError>
where
    S: Sleeper + ?Sized,
{
    let end = match run.today {
        Some(day) => day
            .checked_sub_days(Days::new(1))
            .ok_or(RollupError::DayUnderflow { day })?,
        None => yesterday(run.clock)?,
    };
    let Some(client) = open(run.store).await? else {
        tracing::warn!("PTU backfill: no database is connected, skipping");
        return Ok(result_of(end, end, 0, 0, 0));
    };
    let loaded = load_models(&client, run.entries).await?;
    let days = backfill_window(&loaded.models, end);
    let Some((start, last)) = days
        .first()
        .zip(days.last())
        .map(|(first, last)| (*first, *last))
    else {
        return Ok(result_of(end, end, 0, 0, 0));
    };
    let priced = existing_sentinel_keys(&client, start, last).await?;
    let missing = unpriced_charges(&loaded.models, &days, &priced)?;
    if missing.is_empty() {
        return Ok(result_of(start, last, days.len(), 0, 0));
    }
    let rows_written = write_missing(&client, &missing, run.clock, run.sleeper).await;
    let rows_failed = missing.len().saturating_sub(rows_written);
    tracing::info!(
        "PTU backfill for {start} to {last}: {} unpriced charge(s) found, {rows_written} written, {rows_failed} failed",
        missing.len()
    );
    Ok(result_of(
        start,
        last,
        days.len(),
        rows_written,
        rows_failed,
    ))
}

async fn write_missing<S>(
    client: &Client,
    missing: &[(NaiveDate, PtuCharge)],
    clock: &dyn WallClock,
    sleeper: &mut S,
) -> usize
where
    S: Sleeper + ?Sized,
{
    let mut rows_written = 0_usize;
    for (day, charge) in missing {
        if write_one(client, charge, *day, clock, sleeper).await {
            rows_written = rows_written.saturating_add(1);
        }
    }
    rows_written
}

// A day is priced when its (team, deployment id, day) sentinel row exists, so a rename or a new rate never files a second charge.
fn unpriced_charges(
    models: &[PtuModel],
    days: &[NaiveDate],
    priced: &BTreeSet<SentinelKey>,
) -> Result<Vec<(NaiveDate, PtuCharge)>, ChargeError> {
    let mut missing = Vec::new();
    for day in days {
        for charge in aggregate_charges(models, *day)? {
            let key = SentinelKey {
                team_id: charge.team_id.clone(),
                model: charge.model_id.clone(),
                date: day.to_string(),
            };
            if !priced.contains(&key) {
                missing.push((*day, charge));
            }
        }
    }
    Ok(missing)
}

const fn result_of(
    start: NaiveDate,
    end: NaiveDate,
    days_scanned: usize,
    rows_written: usize,
    rows_failed: usize,
) -> BackfillResult {
    BackfillResult {
        start,
        end,
        days_scanned,
        rows_written,
        rows_failed,
    }
}
