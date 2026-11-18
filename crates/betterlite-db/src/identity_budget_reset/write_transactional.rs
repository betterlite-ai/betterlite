use std::convert::Infallible;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::budget_reset_policy::retry::RetryMode;
use crate::database_faults::reconnect::{DbReconnect, call_with_db_reconnect_retry};
use crate::postgres_store::session::PostgresStore;
use crate::spend_reset_unit_of_work::batch::SpendResetBatch;
use crate::spend_reset_unit_of_work::{SpendResetUnitOfWork, spend_reset_unit_of_work};

use super::faults::{Fault, write_fault};
use super::phase::IdentityPhase;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct ResetWrite {
    pub identifier: String,
    pub budget_reset_at: Option<DateTime<Utc>>,
    pub spend_decrement: Decimal,
}

// _write_key_reset_updates and _write_user_reset_updates queue every row in one spend_reset_unit_of_work and replay only a connect failure.
pub(crate) async fn write_resets<Reconnect>(
    store: &PostgresStore,
    phase: IdentityPhase,
    writes: Vec<ResetWrite>,
    reconnect: Option<&mut Reconnect>,
) -> Result<(), Fault>
where
    Reconnect: DbReconnect + Send,
{
    let policy = RetryMode::Write.policy(phase.write_reason());
    let rows = writes.as_slice();
    let attempt = move || commit_resets(store, phase, rows);
    call_with_db_reconnect_retry(reconnect, attempt, policy).await
}

async fn commit_resets(
    store: &PostgresStore,
    phase: IdentityPhase,
    writes: &[ResetWrite],
) -> Result<(), Fault> {
    let outcome = spend_reset_unit_of_work(
        || SpendResetBatch {
            store: store.clone(),
        },
        async move |unit: &mut SpendResetUnitOfWork| -> Result<(), Infallible> {
            queue_resets(phase, unit, writes);
            Ok(())
        },
    )
    .await;
    outcome.map_err(write_fault)
}

fn queue_resets(phase: IdentityPhase, unit: &SpendResetUnitOfWork, writes: &[ResetWrite]) {
    for write in writes {
        match phase {
            IdentityPhase::Keys => unit.keys.queue_spend_reset(
                &write.identifier,
                write.budget_reset_at,
                write.spend_decrement,
            ),
            IdentityPhase::Users => unit.users.queue_spend_reset(
                &write.identifier,
                write.budget_reset_at,
                write.spend_decrement,
            ),
        }
    }
}

mod tests;
