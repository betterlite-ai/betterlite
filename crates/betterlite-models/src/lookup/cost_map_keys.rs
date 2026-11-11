use indexmap::IndexMap;

use crate::decisions::Json;

pub type JsonMap = IndexMap<String, Json>;

// A deployment is priced under model_info.base_model, then litellm_params.base_model, then its model.
#[must_use]
pub fn cost_map_backend_model<'map>(
    litellm_params: &'map JsonMap,
    model_info: &'map JsonMap,
) -> Option<&'map str> {
    let info_base_model = model_info
        .get("base_model")
        .and_then(text)
        .filter(|base_model| !base_model.is_empty());
    if let Some(base_model) = info_base_model {
        return Some(base_model);
    }
    let params_base_model = litellm_params
        .get("base_model")
        .and_then(text)
        .filter(|base_model| !base_model.is_empty());
    if let Some(base_model) = params_base_model {
        return Some(base_model);
    }
    litellm_params.get("model").and_then(text)
}

// A responses/ backend registers twice, once under its full name and once without the responses/ prefix.
#[must_use]
pub fn backend_cost_map_keys(model: &str, provider: Option<&str>) -> Vec<String> {
    let backend_key = provider.map_or_else(|| model.to_owned(), |name| format!("{name}/{model}"));
    if backend_key.contains("responses/") {
        let twin = backend_key.replace("responses/", "");
        return vec![backend_key, twin];
    }
    vec![backend_key]
}

fn text(value: &Json) -> Option<&str> {
    match value {
        Json::Text(text) => Some(text),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

mod tests;
