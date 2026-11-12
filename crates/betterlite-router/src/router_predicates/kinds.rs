use betterlite_models::decisions::Json;

use super::JsonMap;
use super::map_values::text_value;

#[must_use]
pub fn is_auto_router(params: &JsonMap, classify: fn(&str) -> Option<&'static str>) -> bool {
    if classify(model_name(params)) == Some("semantic") {
        return true;
    }
    false
}

#[must_use]
pub fn is_complexity_router(params: &JsonMap, classify: fn(&str) -> Option<&'static str>) -> bool {
    if classify(model_name(params)) == Some("complexity") {
        return true;
    }
    false
}

#[must_use]
pub fn is_adaptive_router(params: &JsonMap, classify: fn(&str) -> Option<&'static str>) -> bool {
    if classify(model_name(params)) == Some("adaptive") {
        return true;
    }
    false
}

#[must_use]
pub fn joins_adaptive_routing(
    params: &JsonMap,
    classify: fn(&str) -> Option<&'static str>,
) -> bool {
    if is_adaptive_router(params, classify) {
        return true;
    }
    if !is_complexity_router(params, classify) {
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

fn model_name(litellm_params: &JsonMap) -> &str {
    match litellm_params.get("model") {
        Some(Json::Text(text)) => text,
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => "",
    }
}
