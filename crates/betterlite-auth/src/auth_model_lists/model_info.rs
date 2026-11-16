use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::auth_model_lists::parameters::{ModelParameters, ParameterError};
use crate::auth_model_lists::wildcard::{ExpansionContext, get_known_models_from_wildcard};
use crate::auth_model_patterns::pattern::is_wildcard_pattern;
const MODEL_NAME_FIELD: &str = "model_name";
const LITELLM_PARAMS_FIELD: &str = "litellm_params";
const MODEL_FIELD: &str = "model";
#[must_use]
pub fn expand_wildcard_deployments(
    deployments: &[JsonObject],
    context: &ExpansionContext<'_>,
) -> Vec<JsonObject> {
    let mut expanded: Vec<JsonObject> = Vec::with_capacity(deployments.len());
    for deployment in deployments {
        let model_name = text_field(deployment, MODEL_NAME_FIELD);
        let parameters_object = match deployment.get(LITELLM_PARAMS_FIELD) {
            Some(Json::Object(value)) => value.clone(),
            _ => JsonObject::new(),
        };
        let litellm_model = text_field(&parameters_object, MODEL_FIELD);
        let wildcard_pattern = if is_wildcard_pattern(&model_name) && model_name.contains('/') {
            model_name
        } else if is_wildcard_pattern(&model_name) && is_wildcard_pattern(&litellm_model) {
            litellm_model
        } else if is_wildcard_pattern(&model_name) {
            model_name
        } else {
            expanded.push(deployment.clone());
            continue;
        };
        let Ok(parameters) = parse_parameters(&parameters_object) else {
            expanded.push(deployment.clone());
            continue;
        };
        let names = get_known_models_from_wildcard(&wildcard_pattern, parameters.as_ref(), context);
        if names.is_empty() {
            expanded.push(deployment.clone());
            continue;
        }
        for name in names {
            let mut row = deployment.clone();
            row.insert(MODEL_NAME_FIELD.to_owned(), Json::Text(name.clone()));
            if let Some(Json::Object(params)) = row.get_mut(LITELLM_PARAMS_FIELD) {
                params.insert(MODEL_FIELD.to_owned(), Json::Text(name));
            }
            expanded.push(row);
        }
    }
    expanded
}
// absent or non-object parameters read as none; invalid typed parameters keep the row unexpanded.
fn parse_parameters(object: &JsonObject) -> Result<Option<ModelParameters>, ParameterError> {
    if object.is_empty() {
        return Ok(None);
    }
    ModelParameters::parse(object).map(Some)
}
// `str(row.get(field) or "")`: a missing or non-text value reads as the empty string.
fn text_field(object: &JsonObject, field: &str) -> String {
    match object.get(field) {
        Some(Json::Text(value)) => value.clone(),
        _ => String::new(),
    }
}
