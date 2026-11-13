use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::LoggingState;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CallbackKind {
    #[default]
    External,
    // the proxy's own cost and spend bookkeeping, which must still run for a no-log request
    ProxyInternal,
}

#[must_use]
pub fn should_run_callback(
    state: &LoggingState,
    callback: CallbackKind,
    litellm_params: &IndexMap<String, Json>,
) -> bool {
    if state.callback_policy.global_disable_no_log_param {
        return true;
    }
    if matches!(litellm_params.get("no-log"), Some(Json::Bool(true)))
        && callback != CallbackKind::ProxyInternal
    {
        return false;
    }
    true
}
