use betterlite_prompts::arguments::json::{Json, Object};

use crate::openai_base_config::{CatalogCapabilities, OpenAiConfig};

const EXTRA_PARAMS: [&str; 2] = ["thinking", "reasoning_effort"];
const THINKING: &str = "thinking";
const REASONING_EFFORT: &str = "reasoning_effort";
const ENABLED: &str = "enabled";
const DISABLED: &str = "disabled";
const NONE: &str = "none";

#[must_use]
pub fn supported_openai_params(
    model: &str,
    config: &OpenAiConfig,
    catalog: &CatalogCapabilities,
) -> Vec<&'static str> {
    let mut params = config.supported_params(model, catalog);
    params.extend(EXTRA_PARAMS);
    params
}

pub fn map_openai_params(
    model: &str,
    requested: &Object,
    existing: &mut Object,
    config: &OpenAiConfig,
    catalog: &CatalogCapabilities,
) {
    let supported = supported_openai_params(model, config, catalog);
    for (name, value) in requested {
        if supported.contains(&name.as_str()) {
            existing.insert(name.clone(), value.clone());
        }
    }
    let thinking = existing.shift_remove(THINKING);
    let effort = existing.shift_remove(REASONING_EFFORT);
    if let Some(kind) = selected_thinking(thinking.as_ref(), effort.as_ref()) {
        let mut object = Object::new();
        object.insert("type".to_owned(), Json::Text(kind.to_owned()));
        existing.insert(THINKING.to_owned(), Json::Object(object));
    }
}

// The pinned map pops both names first, so a malformed thinking object still lets reasoning_effort decide
fn selected_thinking(thinking: Option<&Json>, effort: Option<&Json>) -> Option<&'static str> {
    if let Some(Json::Object(fields)) = thinking
        && let Some(kind) = fields.get("type").and_then(Json::as_str)
        && (kind == ENABLED || kind == DISABLED)
    {
        return Some(if kind == ENABLED { ENABLED } else { DISABLED });
    }
    match effort {
        None | Some(Json::Null) => None,
        Some(Json::Text(text)) if text == NONE => Some(DISABLED),
        Some(_) => Some(ENABLED),
    }
}
