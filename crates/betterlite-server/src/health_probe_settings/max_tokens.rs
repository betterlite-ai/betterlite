use betterlite_config::sections::health_probe_policy::Settings as ProbePolicy;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::model_info::HealthCheckModelInfo;
use super::model_settings::ModelHealthSettings;

const DEFAULT_PROBE_MAX_TOKENS: i64 = 16;

#[must_use]
pub fn probe_model_string(litellm_params: &JsonObject) -> &str {
    match litellm_params.get("model") {
        Some(Json::Text(model)) => model,
        _ => "",
    }
}

#[must_use]
pub fn resolve_max_tokens<M>(
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
    let is_wildcard = probe_model_string(litellm_params).contains('*');
    if !is_wildcard {
        let is_reasoning = models.supports_reasoning(probe_model_string(litellm_params));
        if health.max_tokens_reasoning.is_some() || health.max_tokens_non_reasoning.is_some() {
            if is_reasoning && let Some(tokens) = health.max_tokens_reasoning {
                return Some(tokens);
            }
            if !is_reasoning && let Some(tokens) = health.max_tokens_non_reasoning {
                return Some(tokens);
            }
        }
        if is_reasoning && let Some(tokens) = policy.max_tokens_reasoning {
            return Some(tokens);
        }
    }
    if let Some(tokens) = policy.max_tokens {
        return Some(tokens);
    }
    if !is_wildcard {
        return Some(DEFAULT_PROBE_MAX_TOKENS);
    }
    None
}
