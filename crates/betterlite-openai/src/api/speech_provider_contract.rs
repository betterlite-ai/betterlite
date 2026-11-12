use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::openai_base_config::Object;

// LiteLLM's TextToSpeechRequestData is total=False: every key is optional and no key excludes another
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextToSpeechRequestData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dict_body: Option<Object>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssml_body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<IndexMap<String, String>>,
}

// map_openai_params takes the voice as `str | dict | None`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SpeechVoice {
    Name(String),
    Options(Object),
}

mod tests;
