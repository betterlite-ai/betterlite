use serde_json::{Map, Value};

// LiteLLM's _extract_model_from_vertex_ai_setup: a direct model field wins, and setup.model is only read when it is absent
#[must_use]
pub fn extract_vertex_setup_model(setup_response: &Map<String, Value>) -> Option<String> {
    let model_path = match setup_response.get("model") {
        Some(model) => model,
        None => setup_response
            .get("setup")
            .and_then(Value::as_object)?
            .get("model")?,
    };
    let (_prefix, model_name) = model_path.as_str()?.rsplit_once("/models/")?;
    Some(model_name.to_owned())
}
