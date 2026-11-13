use crate::arguments::json::{Json, Object};

use super::INTEGRATION_NAME;

const INJECTION_POINTS: &str = "cache_control_injection_points";

#[must_use]
pub fn should_use_cache_control(non_default_params: &Object) -> bool {
    // LiteLLM reads the key with a None default, so null, an empty list and an empty map all say no
    non_default_params
        .get(INJECTION_POINTS)
        .is_some_and(Json::is_truthy)
}

#[must_use]
pub fn cache_control_custom_logger<Logger, Init>(
    non_default_params: &Object,
    init: Init,
) -> Option<Logger>
where
    Init: FnOnce(&'static str) -> Option<Logger>,
{
    if !should_use_cache_control(non_default_params) {
        return None;
    }
    init(INTEGRATION_NAME)
}
