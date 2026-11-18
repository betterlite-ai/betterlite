use std::collections::BTreeSet;

use chrono::{Days, NaiveDate};
use uuid::Uuid;

use super::RollupError;
use super::open;
use super::yesterday;
use crate::database_failures::metrics::WallClock;
use crate::postgres_store::session::PostgresStore;
use crate::ptu::charges::deployment::RouterEntry;
use crate::ptu::charges::pricing::{ChargeError, PtuCharge, PtuModel, aggregate_charges};
use crate::ptu::charges::records::BackfillResult;
use crate::ptu::charges::window::backfill_window;
use crate::ptu::store::models::load_models;
use crate::ptu::store::sentinel::{
    ChargeWrite, SentinelKey, existing_sentinel_keys, write_charge_with_retry,
};
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
    let BackfillRun {
        store,
        entries,
        clock,
        sleeper,
        today,
    } = run;
    let end = match today {
        Some(day) => day
            .checked_sub_days(Days::new(1))
            .ok_or(RollupError::DayUnderflow { day })?,
        None => yesterday(clock)?,
    };
    let Some(client) = open(store).await? else {
        tracing::warn!("PTU backfill: no database is connected, skipping");
        return Ok(empty_window(end));
    };
    let loaded = load_models(&client, entries).await?;
    let days = backfill_window(&loaded.models, end);
    let Some((start, last)) = days.first().zip(days.last()).map(|(a, b)| (*a, *b)) else {
        return Ok(empty_window(end));
    };
    let priced = existing_sentinel_keys(&client, start, last).await?;
    let missing = unpriced_charges(&loaded.models, &days, &priced)?;
    if missing.is_empty() {
        return Ok(BackfillResult {
            start,
            end: last,
            days_scanned: days.len(),
            rows_written: 0,
            rows_failed: 0,
        });
    }
    let mut rows_written = 0_usize;
    for (day, charge) in &missing {
        let write = ChargeWrite {
            charge,
            date: *day,
            id: Uuid::new_v4(),
            updated_at: clock.now(),
        };
        if write_charge_with_retry(&client, &mut *sleeper, &write).await {
            rows_written = rows_written.saturating_add(1);
        }
    }
    let rows_failed = missing.len().saturating_sub(rows_written);
    let found = missing.len();
    tracing::info!(
        "PTU backfill for {start} to {last}: {found} unpriced charge(s) found, {rows_written} written, {rows_failed} failed"
    );
    Ok(BackfillResult {
        start,
        end: last,
        days_scanned: days.len(),
        rows_written,
        rows_failed,
    })
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

fn empty_window(end: NaiveDate) -> BackfillResult {
    BackfillResult {
        start: end,
        end,
        days_scanned: 0,
        rows_written: 0,
        rows_failed: 0,
    }
}
