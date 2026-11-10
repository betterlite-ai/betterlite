use indexmap::IndexMap;
use serde_json::Value;

use crate::openai_base_config::Object;

pub const SUPPORTED_PARAMS: &[&str] = &[
    "suffix",
    "max_tokens",
    "max_completion_tokens",
    "top_p",
    "frequency_penalty",
    "presence_penalty",
    "stop",
    "stream",
    "stream_options",
];

pub struct InceptionTextCompletionConfig;

impl InceptionTextCompletionConfig {
    // the loop runs in input order, so a later spelling of the limit wins, and unsupported options are skipped
    pub fn map_params(non_default: &IndexMap<String, Value>, existing: &mut Object) {
        for (name, value) in non_default {
            if name == "max_completion_tokens" {
                existing.insert("max_tokens".to_owned(), value.clone());
                continue;
            }
            if SUPPORTED_PARAMS.contains(&name.as_str()) {
                existing.insert(name.clone(), value.clone());
            }
        }
    }
}
