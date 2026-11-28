use std::collections::BTreeMap;

use betterlite_core::api::responses_mcp_results::McpToolResult;
use betterlite_core::api::responses_provider_selection::ParameterMap;
use betterlite_core::api::responses_tool_namespaces::values::Fields;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::response_input::{ResponsesInput, ResponsesItem, ResponsesTool};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    RequestParams {
        scenarios: Vec<RequestScenario>,
    },
    ToolExecutionEvents {
        request: RequestSpec,
        #[serde(default)]
        tool_calls: Vec<BTreeMap<String, Json>>,
        tool_results: Vec<McpToolResult>,
        #[serde(default)]
        ids: Vec<String>,
    },
    CallParameterStreaming {
        request: RequestSpec,
        #[serde(default)]
        call_params: ParameterMap,
        #[serde(default)]
        should_auto_execute: bool,
        original_stream: bool,
        #[serde(default)]
        ids: Vec<String>,
    },
    AppendOutput {
        response: Box<ResponsesApiResponse>,
        #[serde(default)]
        mcp_tools_fetched: Vec<Json>,
        tool_results: Vec<McpToolResult>,
        #[serde(default)]
        ids: Vec<String>,
    },
    ProviderCallError {
        request: RequestSpec,
        #[serde(default)]
        call_params: ParameterMap,
        #[serde(default)]
        follow_up_input: Vec<ResponsesItem>,
        status: u16,
        #[serde(default)]
        ids: Vec<String>,
    },
}

#[derive(Debug, Deserialize)]
pub(crate) struct RequestSpec {
    pub(super) input: ResponsesInput,
    pub(super) model: String,
    #[serde(default)]
    pub(super) tools: Option<Vec<ResponsesTool>>,
    #[serde(default)]
    pub(super) previous_response_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RequestScenario {
    pub(super) request: RequestSpec,
    #[serde(default)]
    pub(super) call_params: ParameterMap,
    #[serde(default)]
    pub(super) kwargs: ParameterMap,
    #[serde(default)]
    pub(super) auth: AuthSpec,
    #[serde(default)]
    pub(super) mcp_tools: Vec<Fields>,
    #[serde(default)]
    pub(super) mcp_events: Vec<BTreeMap<String, Value>>,
    #[serde(default)]
    pub(super) tool_server_map: BTreeMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum AuthSpec {
    #[default]
    None,
    Direct {
        user_id: String,
    },
    Metadata {
        #[serde(default)]
        user_id: Option<String>,
    },
}
