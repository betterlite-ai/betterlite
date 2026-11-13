use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpToolResult {
    pub content: Vec<Json>,
    pub structured_content: Option<Json>,
    pub is_error: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct McpHiddenParams {
    // ast-grep-ignore: no-float-money
    pub response_cost: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct McpPostCallResponse {
    pub mcp_tool_call_response: Vec<Json>,
    pub hidden_params: McpHiddenParams,
}

pub struct McpToolHookCall<'call> {
    pub kwargs: &'call IndexMap<String, Json>,
    pub response: &'call mut McpPostCallResponse,
    pub original: &'call mut McpToolResult,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
}
