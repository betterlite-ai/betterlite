use std::collections::BTreeMap;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::Number;

use super::error::PetalsSetupError;

// LiteLLM reads DEFAULT_MAX_TOKENS when it imports (constants.py L90) and uses it as litellm.max_tokens, which is the petals class default (transformation.py L43); an unset or blank value is 4096
pub const MAX_NEW_TOKENS_ENV: &str = "DEFAULT_MAX_TOKENS";
pub const SUPPORTED_OPENAI_PARAMS: [&str; 4] = ["max_tokens", "temperature", "top_p", "stream"];

// PetalsConfig.validate_environment returns {}: a petals request carries no provider headers and drops the caller's (transformation.py L123-L133)
pub const REQUEST_HEADERS: Headers = BTreeMap::new();

// max_new_tokens is the caller's resolved default, the way PredibaseConfig takes its own and triton takes max_tokens_default
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PetalsConfig {
    pub max_length: Option<Number>,
    pub max_new_tokens: i64,
    pub do_sample: Option<bool>,
    pub temperature: Option<Number>,
    pub top_k: Option<Number>,
    pub top_p: Option<Number>,
    pub repetition_penalty: Option<Number>,
}

pub fn max_new_tokens_default(
    environment: &dyn EnvironmentLookup,
) -> Result<i64, PetalsSetupError> {
    let text = environment.lookup(MAX_NEW_TOKENS_ENV).unwrap_or_default();
    let configured = betterlite_env::number::<i64>(MAX_NEW_TOKENS_ENV, &text)
        .map_err(|source| PetalsSetupError::Environment { source })?;
    Ok(configured.unwrap_or(4096))
}

impl PetalsConfig {
    #[must_use]
    pub fn get_config(&self) -> Object {
        let mut config = Object::new();
        if let Some(max_length) = &self.max_length {
            config.insert(String::from("max_length"), Json::Number(max_length.clone()));
        }
        config.insert(
            String::from("max_new_tokens"),
            Json::Number(Number::from(self.max_new_tokens)),
        );
        if let Some(do_sample) = self.do_sample {
            config.insert(String::from("do_sample"), Json::Bool(do_sample));
        }
        if let Some(temperature) = &self.temperature {
            config.insert(
                String::from("temperature"),
                Json::Number(temperature.clone()),
            );
        }
        if let Some(top_k) = &self.top_k {
            config.insert(String::from("top_k"), Json::Number(top_k.clone()));
        }
        if let Some(top_p) = &self.top_p {
            config.insert(String::from("top_p"), Json::Number(top_p.clone()));
        }
        if let Some(repetition_penalty) = &self.repetition_penalty {
            config.insert(
                String::from("repetition_penalty"),
                Json::Number(repetition_penalty.clone()),
            );
        }
        config
    }
}

pub fn map_openai_params(non_default: &Object, options: &mut Object) {
    for (name, value) in non_default {
        match name.as_str() {
            "max_tokens" => {
                options.insert(String::from("max_new_tokens"), value.clone());
            }
            "temperature" | "top_p" | "stream" => {
                options.insert(name.clone(), value.clone());
            }
            _ => {}
        }
    }
}
