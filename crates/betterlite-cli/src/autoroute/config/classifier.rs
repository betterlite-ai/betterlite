use serde::{Deserialize, Serialize};

pub const DEFAULT_CLASSIFIER_TIMEOUT_MS: u64 = 3000;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LlmClassifierConfig {
    pub model: String,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

const fn default_timeout_ms() -> u64 {
    DEFAULT_CLASSIFIER_TIMEOUT_MS
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassifierType {
    Llm,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClassifierChoice {
    Heuristic,
    Llm(LlmClassifierConfig),
}
