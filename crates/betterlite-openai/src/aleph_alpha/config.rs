use serde_json::Value;
use thiserror::Error;

use crate::openai_base_config::Object;

// the settings AlephAlphaConfig declares, so a caller's override cannot introduce a key LiteLLM would not read
const CONFIG_KEYS: [&str; 31] = [
    "maximum_tokens",
    "minimum_tokens",
    "echo",
    "temperature",
    "top_k",
    "top_p",
    "presence_penalty",
    "frequency_penalty",
    "sequence_penalty",
    "sequence_penalty_min_length",
    "repetition_penalties_include_prompt",
    "repetition_penalties_include_completion",
    "use_multiplicative_presence_penalty",
    "use_multiplicative_frequency_penalty",
    "use_multiplicative_sequence_penalty",
    "penalty_bias",
    "penalty_exceptions_include_stop_sequences",
    "best_of",
    "n",
    "logit_bias",
    "log_probs",
    "stop_sequences",
    "tokens",
    "raw_completion",
    "disable_optimizations",
    "completion_bias_inclusion",
    "completion_bias_exclusion",
    "completion_bias_inclusion_first_token_only",
    "completion_bias_exclusion_first_token_only",
    "contextual_control_threshold",
    "control_log_additive",
];

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ConfigError {
    #[error("Aleph Alpha has no {key} config setting")]
    UnknownSetting { key: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlephAlphaConfig {
    defaults: Object,
}

impl AlephAlphaConfig {
    // LiteLLM's class default is litellm.max_tokens, and only a non-None constructor value replaces it
    pub fn new(max_tokens: i64, overrides: Object) -> Result<Self, ConfigError> {
        let mut defaults =
            Object::from_iter([(String::from("maximum_tokens"), Value::from(max_tokens))]);
        for (key, value) in overrides {
            if !CONFIG_KEYS.contains(&key.as_str()) {
                return Err(ConfigError::UnknownSetting { key });
            }
            if value.is_null() {
                continue;
            }
            defaults.insert(key, value);
        }
        Ok(Self { defaults })
    }

    // LiteLLM fills a default only when the option key is absent, whatever value the option holds
    pub fn merge_defaults(&self, options: &mut Object) {
        for (key, value) in &self.defaults {
            if options.contains_key(key) {
                continue;
            }
            options.insert(key.clone(), value.clone());
        }
    }
}
