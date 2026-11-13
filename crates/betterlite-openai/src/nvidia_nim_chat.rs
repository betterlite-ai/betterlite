mod tests;

use crate::openai_base_config::{Object, OpenAiConfig};

const GEMMA: [&str; 6] = [
    "stream",
    "temperature",
    "top_p",
    "max_tokens",
    "stop",
    "seed",
];
const NEMOTRON_INSTRUCT: [&str; 5] = [
    "stream",
    "temperature",
    "top_p",
    "max_tokens",
    "max_completion_tokens",
];
const NEMOTRON_REWARD: [&str; 1] = ["stream"];
const CODEGEMMA: [&str; 8] = [
    "stream",
    "temperature",
    "top_p",
    "frequency_penalty",
    "presence_penalty",
    "max_tokens",
    "max_completion_tokens",
    "stop",
];
const DEFAULT: [&str; 13] = [
    "stream",
    "temperature",
    "top_p",
    "frequency_penalty",
    "presence_penalty",
    "max_tokens",
    "max_completion_tokens",
    "stop",
    "seed",
    "tools",
    "tool_choice",
    "parallel_tool_calls",
    "response_format",
];

const MAX_COMPLETION_TOKENS: &str = "max_completion_tokens";
const MAX_TOKENS: &str = "max_tokens";

// LiteLLM's NvidiaNimConfig replaces the base allowlist and the base mapping instead of extending them
#[derive(Debug, Clone)]
pub struct NvidiaNimConfig {
    pub base: OpenAiConfig,
}

impl NvidiaNimConfig {
    #[must_use]
    pub fn supported_params(model: &str) -> Vec<&'static str> {
        match model {
            "google/recurrentgemma-2b"
            | "google/gemma-2-27b-it"
            | "google/gemma-2-9b-it"
            | "gemma-2-9b-it" => GEMMA.to_vec(),
            "nvidia/nemotron-4-340b-instruct" => NEMOTRON_INSTRUCT.to_vec(),
            "nvidia/nemotron-4-340b-reward" => NEMOTRON_REWARD.to_vec(),
            "google/codegemma-1.1-7b" => CODEGEMMA.to_vec(),
            _ => DEFAULT.to_vec(),
        }
    }

    #[must_use]
    pub fn map_params(
        model: &str,
        non_default_params: &Object,
        mut optional_params: Object,
    ) -> Object {
        let supported = Self::supported_params(model);
        for (name, value) in non_default_params {
            if name == MAX_COMPLETION_TOKENS {
                optional_params.insert(MAX_TOKENS.to_owned(), value.clone());
            } else if supported.contains(&name.as_str()) {
                optional_params.insert(name.clone(), value.clone());
            }
        }
        optional_params
    }
}
