use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::openai_base_config::Object;

mod tests;

// LiteLLM's AI21ChatConfig returns this list whatever the model is, and never the OpenAI base list
pub const SUPPORTED_OPENAI_PARAMS: [&str; 10] = [
    "tools",
    "response_format",
    "max_tokens",
    "max_completion_tokens",
    "temperature",
    "stop",
    "n",
    "stream",
    "seed",
    "tool_choice",
];

#[derive(Debug, Error)]
pub enum Ai21ChatError {
    #[error("the AI21 chat settings could not be read: {source}")]
    Settings {
        #[source]
        source: serde_json::Error,
    },
}

// the fields AI21ChatConfig declares plus the temperature and top_p its constructor takes, each absent at first
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Ai21ChatConfig {
    pub tools: Option<Value>,
    pub response_format: Option<Value>,
    pub documents: Option<Value>,
    pub max_tokens: Option<Value>,
    pub temperature: Option<Value>,
    pub top_p: Option<Value>,
    pub stop: Option<Value>,
    #[serde(rename = "n")]
    pub number: Option<Value>,
    pub stream: Option<Value>,
    pub seed: Option<Value>,
    pub tool_choice: Option<Value>,
    pub user: Option<Value>,
}

impl TryFrom<Object> for Ai21ChatConfig {
    type Error = Ai21ChatError;

    fn try_from(settings: Object) -> Result<Self, Self::Error> {
        serde_json::from_value(Value::Object(settings))
            .map_err(|source| Ai21ChatError::Settings { source })
    }
}

impl Ai21ChatConfig {
    // get_config reads the class dict, so a field reaches the caller only when the constructor was given it
    #[must_use]
    pub fn config(&self) -> Object {
        let mut config = Object::new();
        for (name, value) in [
            ("tools", &self.tools),
            ("response_format", &self.response_format),
            ("documents", &self.documents),
            ("max_tokens", &self.max_tokens),
            ("temperature", &self.temperature),
            ("top_p", &self.top_p),
            ("stop", &self.stop),
            ("n", &self.number),
            ("stream", &self.stream),
            ("seed", &self.seed),
            ("tool_choice", &self.tool_choice),
            ("user", &self.user),
        ] {
            if let Some(stored) = value {
                config.insert(name.to_owned(), stored.clone());
            }
        }
        config
    }
}

// AI21 filters by its ten names, then the OpenAI-like mapper renames the token limit its list still carries
pub fn map_openai_params(non_default_params: &Object, optional_params: &mut Object) {
    for (name, value) in non_default_params {
        if SUPPORTED_OPENAI_PARAMS.contains(&name.as_str()) {
            optional_params.insert(name.clone(), value.clone());
        }
    }
    if let Some(limit) = non_default_params.get("max_completion_tokens") {
        optional_params.insert(String::from("max_tokens"), limit.clone());
        optional_params.remove("max_completion_tokens");
    }
}
