use betterlite_models::decisions::Json;

use super::JsonMap;
use super::map_values::text_value;

// Callers compare the kind with "semantic", "complexity" or "adaptive", as router.py's three _is_*_router_deployment do
#[must_use]
pub fn strategy_kind(
    litellm_params: &JsonMap,
    classify: fn(&str) -> Option<&'static str>,
) -> Option<&'static str> {
    match litellm_params.get("model") {
        Some(Json::Text(text)) => classify(text),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => classify(""),
    }
}

#[must_use]
pub fn joins_adaptive_routing(
    params: &JsonMap,
    classify: fn(&str) -> Option<&'static str>,
) -> bool {
    let kind = strategy_kind(params, classify);
    if kind == Some("adaptive") {
        return true;
    }
    if kind != Some("complexity") {
        return false;
    }
    let Some(Json::Object(config)) = params.get("complexity_router_config") else {
        return false;
    };
    if config.is_empty() {
        return false;
    }
    matches!(config.get("adaptive"), Some(Json::Bool(true)))
}

#[must_use]
pub fn deployment_tags(deployment: &JsonMap) -> Vec<String> {
    let Some(Json::Object(params)) = deployment.get("litellm_params") else {
        return Vec::new();
    };
    let Some(Json::List(tags)) = params.get("tags") else {
        return Vec::new();
    };
    tags.iter()
        .filter_map(text_value)
        .map(str::to_owned)
        .collect()
}
