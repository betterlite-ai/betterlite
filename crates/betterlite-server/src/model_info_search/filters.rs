use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

// _is_auto_router_model: every auto_router/* strategy, not only the semantic one.
#[must_use]
pub fn is_auto_router_model(model: &JsonObject) -> bool {
    let Some(Json::Object(params)) = model.get("litellm_params") else {
        return false;
    };
    matches!(
        params.get("model"),
        Some(Json::Text(name)) if name.starts_with("auto_router/")
    )
}

// _model_in_access_group: a stored list of strings, and an exact membership test against it.
#[must_use]
pub fn model_in_access_group(model: &JsonObject, access_group: &str) -> bool {
    let Some(Json::Object(info)) = model.get("model_info") else {
        return false;
    };
    let Some(Json::List(groups)) = info.get("access_groups") else {
        return false;
    };
    groups
        .iter()
        .any(|group| matches!(group, Json::Text(name) if name == access_group))
}

#[must_use]
pub fn matches_model_info_filters(
    model: &JsonObject,
    exclude_auto_routers: Option<bool>,
    access_group: Option<&str>,
    wildcard_only: Option<bool>,
) -> bool {
    if exclude_auto_routers == Some(true) && is_auto_router_model(model) {
        return false;
    }
    if let Some(group) = access_group {
        if !model_in_access_group(model, group) {
            return false;
        }
    }
    if wildcard_only != Some(true) {
        return true;
    }
    let name = match model.get("model_name") {
        Some(Json::Text(name)) => name.as_str(),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            ""
        }
    };
    name.contains('*')
}
