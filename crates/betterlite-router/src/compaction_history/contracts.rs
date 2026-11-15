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

pub enum Conversation<'source> {
    Messages(&'source [ChatMessage]),
    Responses {
        items: &'source [Json],
        tools: &'source [Json],
    },
    Text(&'source str),
}

// chat tools carry a type; the Messages surface sends Anthropic tool params verbatim, with name and input_schema
#[derive(Clone, Copy)]
pub enum ParentTools<'params> {
    Chat(&'params [Tool]),
    Anthropic(&'params [Json]),
}

pub struct ParentPayload<'params> {
    pub model: &'params str,
    pub messages: &'params [ChatMessage],
    pub tools: Option<ParentTools<'params>>,
    pub system: Option<&'params Json>,
    pub user: Option<&'params Json>,
}

pub struct ResponsesPayload<'params> {
    pub model: &'params str,
    pub items: &'params [Json],
    pub tools: &'params [Json],
    pub request: &'params Json,
    pub system: Option<&'params Json>,
    pub user: Option<&'params Json>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum SummaryTools {
    Chat(Vec<Tool>),
    Anthropic(Vec<Json>),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SummaryRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub max_tokens: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<SummaryTools>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<Json>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<Json>,
}

pub struct NativePrefix {
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<Tool>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeCapability {
    Unsupported,
    IncompatibleDefaults,
    Supported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeSupport {
    pub provider: bool,
    pub capability: NativeCapability,
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
