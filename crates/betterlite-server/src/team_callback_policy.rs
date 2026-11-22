use betterlite_logging::callback_config::options::callback_config_error;
use betterlite_logging::callback_config::{CallbackVars, FeatureFlags};

use crate::team_callback_policy::error::TeamCallbackError;

pub mod audit;
pub mod error;
pub mod redaction;
pub mod resolve;

mod tests;

pub const CALLBACK_VARS_REDACTED: &str = "***REDACTED***";

// _validate_team_callback: B246's first rejection becomes HTTPException(400, detail={"error": message})
pub fn validate_team_callback(
    callback_name: &str,
    callback_vars: &CallbackVars,
    flags: FeatureFlags,
) -> Result<(), TeamCallbackError> {
    callback_config_error(Some(callback_name), callback_vars, flags)
        .map_err(|source| TeamCallbackError::Config { source })
}
