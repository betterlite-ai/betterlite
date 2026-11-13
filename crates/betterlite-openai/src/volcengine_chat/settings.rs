use serde::Deserialize;
use serde_json::Value;

use crate::openai_base_config::Object;

use super::error::VolcEngineChatError;

// LiteLLM's class snapshot: each constructor field starts absent and a given value is kept under its own name
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VolcEngineChatConfig {
    pub frequency_penalty: Option<Value>,
    pub function_call: Option<Value>,
    pub functions: Option<Value>,
    pub logit_bias: Option<Value>,
    pub max_tokens: Option<Value>,
    #[serde(rename = "n")]
    pub number: Option<Value>,
    pub presence_penalty: Option<Value>,
    pub stop: Option<Value>,
    pub temperature: Option<Value>,
    pub top_p: Option<Value>,
    pub response_format: Option<Value>,
}

impl TryFrom<Object> for VolcEngineChatConfig {
    type Error = VolcEngineChatError;

    fn try_from(defaults: Object) -> Result<Self, Self::Error> {
        serde_json::from_value(Value::Object(defaults))
            .map_err(|source| VolcEngineChatError::Settings { source })
    }
}

impl VolcEngineChatConfig {
    // get_config reads the class dict, so a field only reaches the caller when the constructor was given a value
    #[must_use]
    pub fn config(&self) -> Object {
        let mut config = Object::new();
        for (name, value) in [
            ("frequency_penalty", &self.frequency_penalty),
            ("function_call", &self.function_call),
            ("functions", &self.functions),
            ("logit_bias", &self.logit_bias),
            ("max_tokens", &self.max_tokens),
            ("n", &self.number),
            ("presence_penalty", &self.presence_penalty),
            ("stop", &self.stop),
            ("temperature", &self.temperature),
            ("top_p", &self.top_p),
            ("response_format", &self.response_format),
        ] {
            if let Some(stored) = value {
                config.insert(name.to_owned(), stored.clone());
            }
        }
        config
    }
}
