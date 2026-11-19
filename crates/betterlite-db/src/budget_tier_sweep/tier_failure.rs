use std::error::Error;

use crate::budget_reset_policy::phase_failure::EventMetadata;
use crate::identity_budget_reset::faults::describe;

use super::tier_cascade::TierCascadeFailure;
use super::tier_progress::cascade_metadata;

// The failure hook runs under reset_budget_endusers even though this phase resets budget tiers; the call_type is copied as LiteLLM sends it.
pub(crate) const FAILURE_CALL_TYPE: &str = "reset_budget_endusers";

// _budget_cascade_event_metadata(cascade): a read or collect failure carries the empty cascade, so both counts are zero.
#[must_use]
pub(crate) fn failure_metadata<ScheduleFailure>(
    failure: &TierCascadeFailure<ScheduleFailure>,
) -> EventMetadata {
    let budgets = match failure {
        TierCascadeFailure::Collect(_) => 0,
        TierCascadeFailure::Commit { budgets, .. } => *budgets,
    };
    cascade_metadata(budgets, 0)
}

#[must_use]
pub(crate) fn cascade_failure_message<ScheduleFailure>(
    failure: &TierCascadeFailure<ScheduleFailure>,
) -> String
where
    ScheduleFailure: Error,
{
    match failure {
        TierCascadeFailure::Collect(error) => describe(error),
        TierCascadeFailure::Commit { error, .. } => error.to_string(),
    }
}

mod tests;
