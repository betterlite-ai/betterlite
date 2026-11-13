use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatMessage;
use betterlite_router::compaction_history::contracts::{SummaryRequest, Surface};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Function {
    History,
    Compactor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupInput {
    pub model: String,
    pub deployments: Vec<DeploymentInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentInput {
    pub litellm_params: IndexMap<String, Json>,
    #[serde(default)]
    pub max_input_tokens: Option<i64>,
    #[serde(default)]
    pub max_output_tokens: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub function: Function,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub surface: Option<Surface>,
    #[serde(default)]
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub items: Vec<Json>,
    #[serde(default)]
    pub tools: Vec<Json>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub config_model: Option<String>,
    #[serde(default)]
    pub max_tokens: Option<i64>,
    #[serde(default)]
    pub count: Option<i64>,
    #[serde(default)]
    pub groups: Vec<GroupInput>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Ok,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    ContextWindowExceeded,
    BadRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub enum Output {
    History {
        outcome: Outcome,
        instructions: usize,
        prefix: usize,
        tail: usize,
        calls: Vec<String>,
        results: Vec<String>,
        reason: Option<String>,
        status: Option<u16>,
        code: Option<String>,
        kind: Option<FailureKind>,
    },
    Compactor {
        outcome: Outcome,
        model: Option<String>,
        request: Option<SummaryRequest>,
        reason: Option<String>,
        status: Option<u16>,
        code: Option<String>,
        kind: Option<FailureKind>,
    },
}
