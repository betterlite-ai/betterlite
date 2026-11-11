use indexmap::IndexMap;
use serde_json::value::RawValue;

pub type Params = IndexMap<String, Box<RawValue>>;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 3] = ["max_tokens", "max_completion_tokens", "stream"];

// Both token aliases write one key, so the last alias in request order wins
#[must_use]
pub fn map_openai_params(non_default_params: &Params, mut optional_params: Params) -> Params {
    for (param, value) in non_default_params {
        match param.as_str() {
            "max_tokens" | "max_completion_tokens" => {
                optional_params.insert("max_tokens".to_owned(), value.clone());
            }
            "stream" => {
                optional_params.insert("stream".to_owned(), value.clone());
            }
            _ => {}
        }
    }
    optional_params
}

pub fn apply_config_to_params(config: &Params, inference_params: &mut Params) {
    for (key, value) in config {
        if !inference_params.contains_key(key) {
            inference_params.insert(key.clone(), value.clone());
        }
    }
}
