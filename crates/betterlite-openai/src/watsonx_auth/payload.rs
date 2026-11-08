use serde_json::Value;

use super::types::{WatsonxApiParams, WatsonxParams};

#[must_use]
pub fn prepare_payload(model: &str, api_params: &WatsonxApiParams) -> WatsonxParams {
    let mut payload = WatsonxParams::new();
    if model.starts_with("deployment/") {
        return payload;
    }
    payload.insert("model_id".to_owned(), Value::String(model.to_owned()));
    match &api_params.project_id {
        Some(project_id) => {
            payload.insert("project_id".to_owned(), Value::String(project_id.clone()));
        }
        None => {
            payload.insert(
                "space_id".to_owned(),
                api_params
                    .space_id
                    .clone()
                    .map_or(Value::Null, Value::String),
            );
        }
    }
    payload
}
