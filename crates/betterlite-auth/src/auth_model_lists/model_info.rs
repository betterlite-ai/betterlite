use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::auth_model_lists::parameters::ModelParameters;
use crate::auth_model_lists::providers::is_wildcard;
use crate::auth_model_lists::wildcard::{ExpansionContext, get_known_models_from_wildcard};

const MODEL_NAME_FIELD: &str = "model_name";
const LITELLM_PARAMS_FIELD: &str = "litellm_params";
const MODEL_FIELD: &str = "model";

// expand_wildcard_deployments_for_model_info: one row per expanded name, every concrete row untouched.
#[must_use]
pub fn expand_wildcard_deployments_for_model_info(
    deployments: &[JsonObject],
    context: &ExpansionContext<'_>,
) -> Vec<JsonObject> {
    let mut expanded: Vec<JsonObject> = Vec::with_capacity(deployments.len());
    for deployment in deployments {
        let model_name = text_field(deployment, MODEL_NAME_FIELD);
        let parameters_object = object_field(deployment, LITELLM_PARAMS_FIELD);
        let litellm_model = text_field(&parameters_object, MODEL_FIELD);
        let wildcard_pattern = if is_wildcard(&model_name) && model_name.contains('/') {
            model_name
        } else if is_wildcard(&model_name) && is_wildcard(&litellm_model) {
            litellm_model
        } else if is_wildcard(&model_name) {
            model_name
        } else {
            expanded.push(deployment.clone());
            continue;
        };
        let parameters = if parameters_object.is_empty() {
            None
        } else {
            match ModelParameters::parse(&parameters_object) {
                Ok(parsed) => Some(parsed),
                Err(_invalid) => {
                    expanded.push(deployment.clone());
                    continue;
                }
            }
        };
        let expanded_names =
            get_known_models_from_wildcard(&wildcard_pattern, parameters.as_ref(), context);
        if expanded_names.is_empty() {
            expanded.push(deployment.clone());
            continue;
        }
        for name in expanded_names {
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

// `str(row.get(field) or "")`: a missing or non-text value reads as the empty string.
fn text_field(object: &JsonObject, field: &str) -> String {
    match object.get(field) {
        Some(Json::Text(value)) => value.clone(),
        _ => String::new(),
    }
}

// `raw if isinstance(raw, dict) else {}`: a missing or non-object value reads as no parameters.
fn object_field(object: &JsonObject, field: &str) -> JsonObject {
    match object.get(field) {
        Some(Json::Object(value)) => value.clone(),
        _ => JsonObject::new(),
    }
}
