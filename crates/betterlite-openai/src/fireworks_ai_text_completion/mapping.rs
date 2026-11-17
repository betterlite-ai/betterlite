use serde_json::Value;

use super::supported::SUPPORTED_OPENAI_PARAMS;
use crate::openai_base_config::Object;

// LiteLLM's map_openai_params for text completion: each supported name is copied as given, a null value included.
pub fn map_openai_params(non_default_params: &[(String, Value)], optional_params: &mut Object) {
    for (name, value) in non_default_params {
        if SUPPORTED_OPENAI_PARAMS.contains(&name.as_str()) {
            optional_params.insert(name.clone(), value.clone());
        }
    }
}
