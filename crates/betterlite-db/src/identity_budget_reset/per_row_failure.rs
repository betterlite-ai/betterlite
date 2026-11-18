use crate::budget_reset_policy::entity_reset::ResetSchedule;
use crate::budget_reset_policy::rollover::RolloverPolicy;
use crate::budget_reset_policy::row_failures::RowFailures;

use super::faults::describe;
use super::phase::IdentityPhase;
use super::rows::DueIdentity;
use super::spend_decrement::reset_due_row;
use super::write_transactional::ResetWrite;

// LiteLLM collects a failed row and keeps going: the rows that did reset are committed, and raising here would cost the rest of the chunk.
pub(crate) fn collect_resets<Schedule>(
    phase: IdentityPhase,
    rows: &[DueIdentity],
    rollover: RolloverPolicy,
    schedule: &Schedule,
) -> (Vec<ResetWrite>, RowFailures)
where
    Schedule: ResetSchedule,
{
    let mut writes = Vec::new();
    let mut failures = RowFailures::default();
    for row in rows {
        match reset_due_row(row, rollover, schedule) {
            Ok(write) => writes.push(write),
            Err(failure) => {
                let message = describe(&failure);
                tracing::error!(
                    row = %row.identifier,
                    error = %message,
                    subject = phase.row_subject(),
                    "Failed to reset budget row"
                );
                failures.record(&row.identifier, &message);
            }
        }
    }
    (writes, failures)
}

mod tests;
