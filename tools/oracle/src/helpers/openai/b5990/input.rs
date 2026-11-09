use std::collections::BTreeMap;

use betterlite_openai::chat::ChatMessage;
use betterlite_openai::huggingface_chat::mapping::ProviderMapping;
use betterlite_openai::openai_base_config::{Headers, Object};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Step {
    CompleteUrl {
        model: String,
        api_base: Option<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
    ValidateEnvironment {
        #[serde(default)]
        headers: Headers,
        api_key: Option<String>,
    },
    TransformRequest {
        model: String,
        messages: Vec<ChatMessage>,
        #[serde(default)]
        optional_params: Object,
        api_base: Option<String>,
        mapping: Option<ProviderMapping>,
    },
}
