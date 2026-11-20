use std::convert::Infallible;

use crate::budget_reset_policy::retry::RetryMode;
use crate::database_faults::reconnect::{DbReconnect, call_with_db_reconnect_retry};
use crate::identity_budget_reset::faults::{Fault, write_fault};
use crate::identity_budget_reset::write_transactional::ResetWrite;
use crate::postgres_store::session::PostgresStore;
use crate::spend_reset_unit_of_work::batch::SpendResetBatch;
use crate::spend_reset_unit_of_work::{SpendResetUnitOfWork, spend_reset_unit_of_work};

pub(crate) const TEAM_WRITE_REASON: &str = "reset_budget_write_teams_failure";

// _write_team_reset_updates: every row lands in one unit of work and a connect fault is the only failure replayed, because then the SQL never ran.
pub(crate) async fn write_team_resets<Reconnect>(
    store: &PostgresStore,
    writes: Vec<ResetWrite>,
    reconnect: Option<&mut Reconnect>,
) -> Result<(), Fault>
where
    Reconnect: DbReconnect + Send,
{
    let policy = RetryMode::Write.policy(TEAM_WRITE_REASON);
    let rows = writes.as_slice();
    let attempt = move || commit_team_resets(store, rows);
    call_with_db_reconnect_retry(reconnect, attempt, policy).await
}

async fn commit_team_resets(store: &PostgresStore, writes: &[ResetWrite]) -> Result<(), Fault> {
    let outcome = spend_reset_unit_of_work(
        || SpendResetBatch {
            store: store.clone(),
        },
        async move |unit: &SpendResetUnitOfWork| -> Result<(), Infallible> {
            for write in writes {
                unit.teams.queue_spend_reset(
                    &write.identifier,
                    write.budget_reset_at,
                    write.spend_decrement,
                );
            }
            Ok(())
        },
    )
    .await;
    outcome.map_err(write_fault)
}

// LiteLLM drops spend:team:{team_id} after the commit, so the next read reseeds from the row that already holds the decrement.
pub(crate) fn team_counter_keys(writes: &[ResetWrite]) -> Vec<String> {
    writes
        .iter()
        .map(|write| {
            let team_id = &write.identifier;
            format!("spend:team:{team_id}")
        })
        .collect()
}

mod tests;
