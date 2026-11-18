use crate::budget_reset_policy::entity_reset::{
    BudgetResetInput, ResetSchedule, reset_budget_values,
};
use crate::budget_reset_policy::rollover::RolloverPolicy;

use super::rows::DueIdentity;
use super::write_transactional::ResetWrite;

// pre_reset_spend - float(updated.spend or 0.0): the write subtracts exactly what the reset removed from this row.
pub(crate) fn reset_due_row<Schedule>(
    row: &DueIdentity,
    rollover: RolloverPolicy,
    schedule: &Schedule,
) -> Result<ResetWrite, Schedule::Failure>
where
    Schedule: ResetSchedule,
{
    let values = reset_budget_values(
        BudgetResetInput {
            spend: Some(row.spend),
            max_budget: row.max_budget,
            budget_duration: Some(row.budget_duration.as_str()),
            budget_reset_at: row.budget_reset_at,
        },
        rollover,
        schedule,
    )?;
    let decrement = row.spend.checked_sub(values.spend).unwrap_or_default();
    Ok(ResetWrite {
        identifier: row.identifier.clone(),
        budget_reset_at: values.budget_reset_at,
        spend_decrement: decrement,
    })
}

mod tests;
