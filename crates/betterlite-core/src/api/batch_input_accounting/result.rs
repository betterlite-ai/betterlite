use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

// The tokens, output estimate and row count one `body.model` accumulated in the file
#[derive(Debug, Clone, Copy, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct BatchInputRowUsage {
    pub total_tokens: i64,
    pub output_tokens: i64,
    pub request_count: i64,
}

// LiteLLM's BatchFileUsage: whole-file totals plus each row model's own share
#[derive(Debug, Clone, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct BatchInputAccountingResult {
    pub total_tokens: i64,
    pub request_count: i64,
    pub output_tokens: i64,
    pub per_model_usage: IndexMap<String, BatchInputRowUsage>,
}
