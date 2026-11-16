use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::fal::vision_chat::error::FalAIError;

pub type Params = IndexMap<String, Json>;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 3] = ["reasoning_effort", "temperature", "top_p"];
const REASONING_DISABLED_EFFORTS: [&str; 2] = ["none", "minimal"];
const REASONING_ENABLED_EFFORTS: [&str; 3] = ["low", "medium", "high"];

// a mapped value replaces an earlier optional one in place, and other input parameters are ignored
pub fn map_openai_params(
    non_default_params: &Params,
    optional_params: Params,
    model: &str,
    drop_params: bool,
) -> Result<Params, FalAIError> {
    let mut mapped = optional_params;
    for (name, value) in non_default_params {
        match name.as_str() {
            "temperature" | "top_p" => {
                mapped.insert(name.clone(), value.clone());
            }
            "reasoning_effort" => match reasoning_enabled(value) {
                Some(enabled) => {
                    mapped.insert("reasoning".to_owned(), Json::Bool(enabled));
                }
                None if drop_params => {}
                None => {
                    return Err(FalAIError::UnsupportedReasoningEffort {
                        value: value.clone(),
                        model: model.to_owned(),
                    });
                }
            },
            _ => {}
        }
    }
    Ok(mapped)
}

fn reasoning_enabled(effort: &Json) -> Option<bool> {
    let Json::Text(text) = effort else {
        return None;
    };
    if REASONING_DISABLED_EFFORTS.contains(&text.as_str()) {
        Some(false)
    } else {
        REASONING_ENABLED_EFFORTS
            .contains(&text.as_str())
            .then_some(true)
    }
}
