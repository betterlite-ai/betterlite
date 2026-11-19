use betterlite_models::decisions::Json;
use serde_json::Number;

use crate::budget_reset_policy::phase_failure::EventMetadata;

use super::enduser_page_truncation::EndUserWalk;

pub(crate) const SUCCESS_CALL_TYPE: &str = "reset_budget_budget_table";

// _budget_cascade_event_metadata: the counts every reset_budget_job event carries, from the cascade and the customer walk.
#[must_use]
pub(crate) fn cascade_metadata(budgets: usize, endusers: usize) -> EventMetadata {
    let mut metadata = EventMetadata::new();
    metadata.insert("num_budgets_found", Json::Number(Number::from(budgets)));
    metadata.insert("num_endusers_found", Json::Number(Number::from(endusers)));
    metadata
}

// The success hook's metadata: the found counts plus what the customer walk reached and whether a failed page cut it short.
#[must_use]
pub(crate) fn success_metadata(budgets: usize, walk: &EndUserWalk) -> EventMetadata {
    let mut metadata = cascade_metadata(budgets, walk.invalidated);
    metadata.insert(
        "num_endusers_updated",
        Json::Number(Number::from(walk.invalidated)),
    );
    metadata.insert("num_endusers_failed", Json::Number(Number::from(0_u8)));
    metadata.insert("enduser_invalidation_truncated", Json::Bool(walk.truncated));
    metadata
}

mod tests;
