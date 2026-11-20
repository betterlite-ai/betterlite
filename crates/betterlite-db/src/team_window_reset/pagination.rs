use chrono::{DateTime, Utc};
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use crate::budget_reset_policy::entity_reset::ResetSchedule;
use crate::budget_reset_policy::phase_draining::ResetLimits;
use crate::budget_reset_policy::retry::RetryMode;
use crate::budget_reset_policy::rollover::RolloverPolicy;
use crate::database_faults::DbError;
use crate::database_faults::reconnect::{DbReconnect, call_with_db_reconnect_retry};
use crate::database_refresh_loop::clock::RefreshClock;
use crate::identity_budget_reset::faults::read_fault;
use crate::postgres_store::errors::backend;
use crate::postgres_store::session::PostgresStore;
use crate::window_spend::store::WindowSpendStore;

use super::cache::WindowCounterCache;
use super::error::WindowResetError;
use super::json_persist::{decode_budget_limits, encode_budget_limits, persist_budget_limits};
use super::source::WindowSource;
use super::window_cache_roll::{WindowDurationSeconds, WindowResetter};

// reset_budget_windows: every source is walked to its end a page at a time; one that raises is logged and abandoned while the next still runs.
pub struct BudgetWindowReset<'ports, Schedule, Cache, Durations, Reconnect, Clock> {
    pub store: PostgresStore,
    pub window_spend: WindowSpendStore,
    pub limits: ResetLimits,
    pub rollover: RolloverPolicy,
    pub schedule: &'ports Schedule,
    pub cache: &'ports Cache,
    pub durations: &'ports Durations,
    pub reconnect: Option<&'ports mut Reconnect>,
    pub clock: &'ports Clock,
}

impl<Schedule, Cache, Durations, Reconnect, Clock>
    BudgetWindowReset<'_, Schedule, Cache, Durations, Reconnect, Clock>
where
    Schedule: ResetSchedule + Sync,
    Cache: WindowCounterCache,
    Durations: WindowDurationSeconds + Sync,
    Reconnect: DbReconnect + Send,
    Clock: RefreshClock,
{
    pub async fn reset_windows(&mut self) {
        let now = self.clock.now();
        for source in WindowSource::ALL {
            if let Err(error) = self.reset_source(source, now).await {
                tracing::error!(
                    source = source.log_subject(),
                    error = %error,
                    "Failed to reset budget windows for {}",
                    source.log_subject()
                );
            }
        }
    }

    async fn reset_source(
        &mut self,
        source: WindowSource,
        now: DateTime<Utc>,
    ) -> Result<(), WindowResetError<Schedule::Failure>> {
        let mut cursor = String::new();
        while let Some(next) = self.reset_page(source, &cursor, now).await? {
            cursor = next;
        }
        Ok(())
    }

    async fn reset_page(
        &mut self,
        source: WindowSource,
        cursor: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<String>, WindowResetError<Schedule::Failure>> {
        let page = PageRequest {
            source,
            cursor,
            limit: self.limits.batch_size,
        };
        let rows = self.read_page(&page).await?;
        let resetter = WindowResetter {
            store: &self.window_spend,
            rollover: self.rollover,
            schedule: self.schedule,
            cache: self.cache,
            durations: self.durations,
            now,
        };
        let store = &self.store;
        for row in &rows {
            let Some(payload) = reset_row(&resetter, source, row).await? else {
                continue;
            };
            let reconnect = self.reconnect.as_deref_mut();
            persist_budget_limits(store, source, &row.id, &payload, reconnect).await?;
        }
        Ok(next_cursor(&rows, self.limits.batch_size))
    }

    async fn read_page(
        &mut self,
        page: &PageRequest<'_>,
    ) -> Result<Vec<WindowRow>, WindowResetError<Schedule::Failure>> {
        let store = &self.store;
        let reconnect = self.reconnect.as_deref_mut();
        let policy = RetryMode::Read.policy(page.source.read_reason());
        let attempt = move || fetch_page(store, page);
        call_with_db_reconnect_retry(reconnect, attempt, policy)
            .await
            .map_err(WindowResetError::Database)
    }
}

struct PageRequest<'page> {
    source: WindowSource,
    cursor: &'page str,
    limit: usize,
}

struct WindowRow {
    id: String,
    budget_limits: Option<String>,
}

async fn fetch_page(
    store: &PostgresStore,
    page: &PageRequest<'_>,
) -> Result<Vec<WindowRow>, DbError> {
    let sql = page.source.page_query();
    let client = store.session().await.map_err(|error| read_fault(&error))?;
    let limit = i64::try_from(page.limit).unwrap_or(i64::MAX);
    let values: [&(dyn ToSql + Sync); 2] = [&page.cursor, &limit];
    let rows = client
        .query(sql.as_str(), &values)
        .await
        .map_err(|error| read_fault(&backend(error)))?;
    let mut decoded = Vec::with_capacity(rows.len());
    for row in &rows {
        decoded.push(decode_row(row)?);
    }
    Ok(decoded)
}

fn decode_row(row: &Row) -> Result<WindowRow, DbError> {
    let id: String = row
        .try_get("row_id")
        .map_err(|error| read_fault(&backend(error)))?;
    let budget_limits: Option<String> = row
        .try_get("budget_limits")
        .map_err(|error| read_fault(&backend(error)))?;
    Ok(WindowRow { id, budget_limits })
}

// _reset_window_page's row body: every window is tried, only the row with one that moved is written back, and the first error ends the source.
async fn reset_row<Schedule, Cache, Durations>(
    resetter: &WindowResetter<'_, Schedule, Cache, Durations>,
    source: WindowSource,
    row: &WindowRow,
) -> Result<Option<String>, WindowResetError<Schedule::Failure>>
where
    Schedule: ResetSchedule + Sync,
    Cache: WindowCounterCache,
    Durations: WindowDurationSeconds + Sync,
{
    let Some(raw) = row.budget_limits.as_deref() else {
        return Ok(None);
    };
    let mut windows = decode_budget_limits(&row.id, raw)?;
    let mut changed = false;
    for window in &mut windows {
        changed |= resetter
            .reset_expired_window(source, &row.id, window)
            .await?;
    }
    if !changed {
        return Ok(None);
    }
    let payload = encode_budget_limits(&row.id, &windows)?;
    Ok(Some(payload))
}

// A short page is the end of the table; a full one hands the last primary key back as the next cursor, so the walk strictly advances.
fn next_cursor(rows: &[WindowRow], batch_size: usize) -> Option<String> {
    if rows.len() < batch_size {
        return None;
    }
    rows.last().map(|row| row.id.clone())
}

mod tests;
