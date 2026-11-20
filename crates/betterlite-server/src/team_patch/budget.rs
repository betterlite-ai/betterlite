use std::collections::BTreeMap;

use betterlite_core::budget_reset::compute_budget_reset_at;
use betterlite_core::duration::error::DurationError;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_logging::private_json::safe_json::DEFAULT_MAX_RECURSE_DEPTH;
use betterlite_logging::private_json::safe_json::safe_dumps;

use crate::key_policy_fields::generate::GenerateError;
use crate::key_policy_fields::generate::generate_budget_windows;
use crate::key_policy_fields::{KeyClock, microsecond_precision, zulu_datetime};
use crate::team_request_contracts::update::UpdateTeamFields;

#[derive(Debug, thiserror::Error)]
pub enum BudgetResetError {
    #[error("{spec} is not a duration")]
    Duration {
        spec: String,
        #[source]
        source: DurationError,
    },
    #[error(transparent)]
    Window(#[from] GenerateError),
    #[error(transparent)]
    Serialization(#[from] PrivateJsonError),
}

// LiteLLM's _set_budget_reset_at: the reset time a supplied duration implies, or the clear an explicit null asks for
pub fn set_budget_reset_at(
    fields: &UpdateTeamFields,
    updated_kv: &mut BTreeMap<String, JsonData>,
    clock: KeyClock<'_>,
) -> Result<(), BudgetResetError> {
    if let Some(spec) = fields.budget_duration.as_deref() {
        let reset = compute_budget_reset_at(spec, clock.reset, clock.now, clock.database).map_err(
            |source| BudgetResetError::Duration {
                spec: spec.to_owned(),
                source,
            },
        )?;
        updated_kv.insert(
            "budget_reset_at".to_owned(),
            JsonData::String(zulu_datetime(microsecond_precision(reset.instant))),
        );
    } else if matches!(updated_kv.get("budget_duration"), Some(JsonData::Null)) {
        // only a duration the caller sent as null clears the column; an absent one leaves it alone
        updated_kv.insert("budget_reset_at".to_owned(), JsonData::Null);
    }
    let Some(windows) = generate_budget_windows(fields.budget_limits.as_deref(), clock)? else {
        return Ok(());
    };
    // the column holds the serialized array, as LiteLLM's json.dumps does
    let serialized = safe_dumps(&windows, DEFAULT_MAX_RECURSE_DEPTH, None)?;
    updated_kv.insert("budget_limits".to_owned(), JsonData::String(serialized));
    Ok(())
}
