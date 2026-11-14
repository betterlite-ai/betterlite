use serde::Deserialize;
use serde_json::Value;

use crate::openai_base_config::Object;

use super::FireworksChatParamsError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextLengthBehavior {
    Error,
    Truncate,
}

impl ContextLengthBehavior {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Truncate => "truncate",
        }
    }
}

// LiteLLM's FireworksAIConfig class attributes: every setting starts absent and only a given value is stored.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FireworksChatSettings {
    pub tools: Option<Value>,
    pub tool_choice: Option<Value>,
    pub max_tokens: Option<Value>,
    pub temperature: Option<Value>,
    pub top_p: Option<Value>,
    pub top_k: Option<Value>,
    pub frequency_penalty: Option<Value>,
    pub presence_penalty: Option<Value>,
    #[serde(rename = "n")]
    pub number: Option<Value>,
    pub stop: Option<Value>,
    pub response_format: Option<Value>,
    pub user: Option<Value>,
    pub logprobs: Option<Value>,
    pub reasoning_effort: Option<Value>,
    pub prompt_truncate_len: Option<Value>,
    #[serde(rename = "context_length_exceeded_behavior")]
    pub context_behavior: Option<ContextLengthBehavior>,
}

impl FireworksChatSettings {
    pub fn with_settings(settings: Object) -> Result<Self, FireworksChatParamsError> {
        serde_json::from_value(Value::Object(settings))
            .map_err(|source| FireworksChatParamsError::Settings { source })
    }

    // get_config reads the class dict, so a field only reaches the caller when the constructor was given a value
    #[must_use]
    pub fn get_config(&self) -> Object {
        let mut config = Object::new();
        for (name, stored) in [
            ("tools", &self.tools),
            ("tool_choice", &self.tool_choice),
            ("max_tokens", &self.max_tokens),
            ("temperature", &self.temperature),
            ("top_p", &self.top_p),
            ("top_k", &self.top_k),
            ("frequency_penalty", &self.frequency_penalty),
            ("presence_penalty", &self.presence_penalty),
            ("n", &self.number),
            ("stop", &self.stop),
            ("response_format", &self.response_format),
            ("user", &self.user),
            ("logprobs", &self.logprobs),
            ("reasoning_effort", &self.reasoning_effort),
            ("prompt_truncate_len", &self.prompt_truncate_len),
        ] {
            if let Some(value) = stored {
                config.insert(name.to_owned(), value.clone());
            }
        }
        if let Some(behavior) = self.context_behavior {
            config.insert(
                String::from("context_length_exceeded_behavior"),
                Value::String(behavior.as_str().to_owned()),
            );
        }
        config
    }
}
