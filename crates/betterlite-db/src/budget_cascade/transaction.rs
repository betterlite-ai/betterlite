use std::convert::Infallible;

use super::BudgetCascade;
use super::enduser_invalidation::queue_enduser_resets;
use super::rollover_order::queue_budget_linked_resets;
use crate::budget_reset_policy::filters::{LINKED_KEYS, SPENT_ROWS};
use crate::postgres_store::session::PostgresStore;
use crate::spend_reset_unit_of_work::batch::{BudgetCascadeBatch, CommitError};
use crate::spend_reset_unit_of_work::queue::SpendRange;
use crate::spend_reset_unit_of_work::{
    BudgetCascadeUnitOfWork, UnitOfWorkError, budget_cascade_unit_of_work,
};

// Zero the gated spend and advance budget_reset_at in one transaction: advancing the window alone would hide the tier from every later tick while its dependents stay pinned at the cap.
pub async fn commit_budget_cascade(
    store: &PostgresStore,
    cascade: &BudgetCascade,
    default_enduser_budget_id: Option<&str>,
) -> Result<(), CommitError> {
    if cascade.budget_ids.is_empty() {
        return Ok(());
    }
    let outcome = budget_cascade_unit_of_work(
        || BudgetCascadeBatch {
            store: store.clone(),
        },
        async move |unit: &mut BudgetCascadeUnitOfWork| -> Result<(), Infallible> {
            queue_budget_linked_resets(&unit.team_memberships, cascade, SpendRange::default());
            queue_budget_linked_resets(&unit.keys, cascade, LINKED_KEYS);
            queue_budget_linked_resets(&unit.organizations, cascade, SPENT_ROWS);
            queue_budget_linked_resets(&unit.tags, cascade, SPENT_ROWS);
            queue_budget_linked_resets(&unit.model_access_groups, cascade, SPENT_ROWS);
            queue_budget_linked_resets(&unit.projects, cascade, SPENT_ROWS);
            queue_enduser_resets(&unit.endusers, cascade, default_enduser_budget_id);
            for reset in &cascade.budget_resets {
                unit.budgets
                    .queue_window_advance(&reset.budget_id, reset.reset_at);
            }
            Ok(())
        },
    )
    .await;
    match outcome {
        Ok(()) => Ok(()),
        Err(UnitOfWorkError::Commit(error)) => Err(error),
        Err(UnitOfWorkError::Body(never)) => match never {},
    }
}

mod tests;
