use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::model_info::HealthCheckModelInfo;
use super::model_settings::ModelHealthSettings;

// Modes whose probe is a chat-style completion call, so adding max_tokens cannot 400
pub const MAX_TOKEN_SUPPORT_MODES: [&str; 3] = ["chat", "completion", "responses"];

// None is the unresolvable mode, which LiteLLM still forwards reasoning_effort for
pub const REASONING_EFFORT_MODES: [Option<&str>; 3] = [None, Some("chat"), Some("completion")];

#[must_use]
pub fn resolve_health_check_mode<M>(
    model_info: &JsonObject,
    litellm_params: &JsonObject,
    models: &M,
) -> Option<String>
where
    M: HealthCheckModelInfo,
{
    if let Some(Json::Text(mode)) = model_info.get("mode") {
        return Some(mode.clone());
    }
    let Some(Json::Text(model)) = litellm_params.get("model") else {
        return None;
    };
    models.mode(model)
}

#[must_use]
pub fn should_inject_max_tokens(health: &ModelHealthSettings, mode: Option<&str>) -> bool {
    if let Some(explicit) = health.supports_max_tokens {
        return explicit;
    }
    MAX_TOKEN_SUPPORT_MODES.contains(&mode.unwrap_or("chat"))
}
