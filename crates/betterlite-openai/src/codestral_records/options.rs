use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const SUPPORTED_PARAMS: [&str; 8] = [
    "suffix",
    "temperature",
    "top_p",
    "max_tokens",
    "max_completion_tokens",
    "stream",
    "seed",
    "stop",
];

// A fresh config exports nothing; top_p and stop appear only when a configured caller supplied them
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodestralSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suffix: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_tokens: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub random_seed: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Value>,
}

// The pairs arrive in encounter order, so the last spelling of a max-token name wins as it does in Python
pub fn map_params(
    non_default_params: &[(String, Value)],
    optional_params: &mut Map<String, Value>,
) {
    for (name, value) in non_default_params {
        let target = match name.as_str() {
            "suffix" | "temperature" | "top_p" | "stop" | "min_tokens" => name.as_str(),
            "max_tokens" | "max_completion_tokens" => "max_tokens",
            "seed" => "random_seed",
            // Python's `value is True` accepts the bool itself only, so 1 does not pass either
            "stream" if *value == Value::Bool(true) => "stream",
            _ => continue,
        };
        optional_params.insert(target.to_owned(), value.clone());
    }
}
