use chrono::{DateTime, Utc};

use crate::budget_reset_policy::retry::RetryMode;
use crate::database_faults::reconnect::{DbReconnect, call_with_db_reconnect_retry};
use crate::identity_budget_reset::faults::{Fault, read_fault};
use crate::identity_budget_reset::rows::{DueIdentity, read_rows};
use crate::postgres_store::session::PostgresStore;

pub(crate) const TEAM_ID_COLUMN: &str = "team_id";
pub(crate) const TEAM_READ_REASON: &str = "reset_budget_read_teams_failure";

// The team find_all: a missing reset time counts as due, so a team carrying a duration but no initialised expiry is never skipped.
pub(crate) const TEAM_DUE_SQL: &str = "SELECT team_id, spend, max_budget, budget_duration, budget_reset_at FROM \"LiteLLM_TeamTable\" WHERE budget_duration IS NOT NULL AND (budget_reset_at IS NULL OR budget_reset_at < $1) LIMIT $2";

pub(crate) async fn read_due_teams<Reconnect>(
    store: &PostgresStore,
    now: DateTime<Utc>,
    limit: usize,
    reconnect: Option<&mut Reconnect>,
) -> Result<Vec<DueIdentity>, Fault>
where
    Reconnect: DbReconnect + Send,
{
    let policy = RetryMode::Read.policy(TEAM_READ_REASON);
    let attempt = move || fetch_due_teams(store, now, limit);
    call_with_db_reconnect_retry(reconnect, attempt, policy).await
}

async fn fetch_due_teams(
    store: &PostgresStore,
    now: DateTime<Utc>,
    limit: usize,
) -> Result<Vec<DueIdentity>, Fault> {
    let client = store.session().await.map_err(|error| read_fault(&error))?;
    read_rows(&client, TEAM_DUE_SQL, TEAM_ID_COLUMN, now, limit)
        .await
        .map_err(|error| read_fault(&error))
}

mod tests;
