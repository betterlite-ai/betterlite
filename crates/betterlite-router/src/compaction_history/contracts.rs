use betterlite_models::decisions::Json;
use betterlite_openai::chat::{ChatMessage, Tool};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Surface {
    Chat,
    Messages,
    Responses,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ModelLimits {
    pub max_input_tokens: Option<i64>,
    pub max_output_tokens: Option<i64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InputBudget {
    pub window: Option<i64>,
    pub available: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OutputAllowance {
    pub max_completion_tokens: Option<i64>,
    pub max_output_tokens: Option<i64>,
    pub max_tokens: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct History {
    pub instruction_count: usize,
    pub prefix_count: usize,
    pub tail_count: usize,
    pub calls: Vec<String>,
    pub results: Vec<String>,
}

pub enum Conversation<'a> {
    Messages(&'a [ChatMessage]),
    Responses {
        items: &'a [Json],
        tools: &'a [Json],
    },
    Text(&'a str),
}

pub struct ParentPayload<'a> {
    pub model: &'a str,
    pub messages: &'a [ChatMessage],
    pub tools: Option<&'a [Tool]>,
    pub system: Option<&'a Json>,
    pub user: Option<&'a Json>,
}

pub struct ResponsesPayload<'a> {
    pub model: &'a str,
    pub items: &'a [Json],
    pub tools: &'a [Json],
    pub request: &'a Json,
    pub system: Option<&'a Json>,
    pub user: Option<&'a Json>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SummaryRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub max_tokens: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<Json>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<Json>,
}

pub struct NativePrefix {
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<Tool>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeSupport {
    pub provider: bool,
    pub supports: bool,
    pub compatible_defaults: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Deployment {
    pub model: String,
    pub custom_llm_provider: Option<String>,
    pub limits: ModelLimits,
    pub native: NativeSupport,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Candidate {
    pub model: String,
    pub deployments: Vec<Deployment>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Selection {
    pub model: String,
    pub deployment: Deployment,
}
