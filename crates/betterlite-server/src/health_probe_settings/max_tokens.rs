use betterlite_config::sections::health_probe_policy::Settings as ProbePolicy;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::model_info::HealthCheckModelInfo;
use super::model_settings::ModelHealthSettings;

const DEFAULT_HEALTH_CHECK_MAX_TOKENS: i64 = 16;

#[must_use]
pub fn deployment_model_string_for_health_check(litellm_params: &JsonObject) -> &str {
    match litellm_params.get("model") {
        Some(Json::Text(model)) => model,
        _ => "",
    }
}

#[must_use]
pub fn resolve_health_check_max_tokens<M>(
    health: &ModelHealthSettings,
    litellm_params: &JsonObject,
    policy: &ProbePolicy,
    models: &M,
) -> Option<i64>
where
    M: HealthCheckModelInfo,
{
    if let Some(explicit) = health.max_tokens {
        return Some(explicit);
    }
    let is_wildcard = deployment_model_string_for_health_check(litellm_params).contains('*');
    if !is_wildcard {
        let is_reasoning =
            models.supports_reasoning(deployment_model_string_for_health_check(litellm_params));
        if health.max_tokens_reasoning.is_some() || health.max_tokens_non_reasoning.is_some() {
            if is_reasoning && let Some(tokens) = health.max_tokens_reasoning {
                return Some(tokens);
            }
            if !is_reasoning && let Some(tokens) = health.max_tokens_non_reasoning {
                return Some(tokens);
            }
        }
        if is_reasoning && let Some(tokens) = policy.background_max_tokens_reasoning {
            return Some(tokens);
        }
    }
    if let Some(tokens) = policy.background_max_tokens {
        return Some(tokens);
    }
    if !is_wildcard {
        return Some(DEFAULT_HEALTH_CHECK_MAX_TOKENS);
    }
    None
}
