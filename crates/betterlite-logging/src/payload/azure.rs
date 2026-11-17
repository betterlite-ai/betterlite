use betterlite_context::chat_config_selection::foundry::{AzureAiRoute, get_azure_ai_route};
use betterlite_models::decisions::Json;

pub const AZURE_ROUTER_MODEL_KEY: &str = "azure_model_router_selected_model";

// AzureFoundryModelInfo.get_model_router_selected_model: the model the router stamped as served.
#[must_use]
pub fn router_selected_model(hidden_params: Option<&Json>) -> Option<String> {
    let value = hidden_params.and_then(|hidden| match hidden {
        Json::Object(fields) => fields.get(AZURE_ROUTER_MODEL_KEY),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    })?;
    match value {
        Json::Text(selected) if !selected.is_empty() => Some(selected.clone()),
        Json::Null
        | Json::Bool(_)
        | Json::Number(_)
        | Json::Text(_)
        | Json::List(_)
        | Json::Object(_) => None,
    }
}

// AzureFoundryModelInfo.is_model_router_call: the stamp first, then the deployment path, then the caller's name.
#[must_use]
pub fn is_model_router_call(requested_model: Option<&str>, hidden_params: Option<&Json>) -> bool {
    if router_selected_model(hidden_params).is_some() {
        return true;
    }
    let deployment_model = hidden_params.and_then(|hidden| match hidden {
        Json::Object(fields) => fields
            .get("litellm_model_name")
            .or_else(|| fields.get("model")),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    });
    let deployment_name = deployment_model.and_then(|candidate| match candidate {
        Json::Text(name) => Some(name.as_str()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    });
    [deployment_name, requested_model]
        .into_iter()
        .flatten()
        .any(|name| get_azure_ai_route(name) == AzureAiRoute::ModelRouter)
}
