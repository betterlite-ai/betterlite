use std::collections::BTreeMap;

use betterlite_core::api::mcp_client_resources::types::{
    GetPromptResult, ReadResourceResult, ServerCapabilities,
};
use serde::Deserialize;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    ListPrompts,
    ListResources,
    ListResourceTemplates,
    GetPrompt,
    ReadResource,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct ErrorInput {
    pub(crate) code: i32,
    pub(crate) message: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct PageInput {
    pub(crate) delay_ms: u64,
    pub(crate) next_cursor: Option<String>,
    pub(crate) items: Vec<serde_json::Value>,
    pub(crate) error: Option<ErrorInput>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct PromptInput {
    pub(crate) name: String,
    pub(crate) arguments: Option<BTreeMap<String, String>>,
    pub(crate) result: Option<GetPromptResult>,
    pub(crate) error: Option<ErrorInput>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct ResourceInput {
    pub(crate) uri: String,
    pub(crate) result: Option<ReadResourceResult>,
    pub(crate) error: Option<ErrorInput>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) operation: Operation,
    pub(crate) client_timeout_ms: u64,
    pub(crate) listing_timeout_ms: u64,
    pub(crate) raise_on_error: bool,
    pub(crate) capabilities: Option<ServerCapabilities>,
    pub(crate) pages: Vec<PageInput>,
    pub(crate) prompt: Option<PromptInput>,
    pub(crate) resource: Option<ResourceInput>,
}
