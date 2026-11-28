use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::response_input::{ResponsesInput, ResponsesItem, ResponsesTool};

use crate::api::responses_mcp_events::event::StreamEvent;
use crate::api::responses_mcp_results::McpToolResult;
use crate::api::responses_mcp_tool_selection::McpToolCall;
use crate::api::responses_provider_selection::ParameterMap;
use crate::api::responses_tool_namespaces::values::Fields;

// uuid.uuid4().hex behind every generated MCP identifier (litellm_proxy_mcp_handler.py:1267, L1326, L1341)
pub type IdSource = Arc<dyn Fn() -> String + Send + Sync>;

#[derive(Debug, Clone, PartialEq)]
pub struct FollowUpCall {
    pub input: ResponsesInput,
    pub model: String,
    pub tools: Option<Vec<ResponsesTool>>,
    pub previous_response_id: Option<String>,
    pub kwargs: ParameterMap,
}

// what _create_mcp_streaming_response (L1145-L1188) passes to MCPEnhancedStreamingIterator with base_iterator=None
#[derive(Debug, Clone)]
pub struct McpStreamingPlan {
    pub request_params: ParameterMap,
    pub mcp_events: Vec<StreamEvent>,
    pub tool_server_map: BTreeMap<String, String>,
    pub mcp_tools_with_litellm_proxy: Vec<Fields>,
    pub auth: Option<UserAPIKeyAuth>,
}

// kwargs.get("litellm_metadata", {}): absent reads as {}, a mapping may hold user_api_key_auth or not, anything else has no .get
#[derive(Debug, Clone)]
pub enum MetadataAuth {
    Absent,
    Mapping(Option<UserAPIKeyAuth>),
    NotMapping,
}

// kwargs.get("user_api_key_auth") or kwargs.get("litellm_metadata", {}).get("user_api_key_auth") (L1185-L1186)
#[derive(Debug, Clone)]
pub struct StreamingAuth {
    pub direct: Option<UserAPIKeyAuth>,
    pub metadata: MetadataAuth,
}

#[derive(Debug, Clone)]
pub struct ResponsesMcpFollowupInput {
    pub input: ResponsesInput,
    pub model: String,
    pub tools: Option<Vec<ResponsesTool>>,
    pub previous_response_id: Option<String>,
    pub call_params: ParameterMap,
    pub kwargs: ParameterMap,
    pub follow_up_input: Vec<ResponsesItem>,
    pub follow_up_response_id: Option<String>,
    pub mcp_tools_with_litellm_proxy: Vec<Fields>,
    pub mcp_discovery_events: Vec<StreamEvent>,
    pub tool_server_map: BTreeMap<String, String>,
    pub auth: StreamingAuth,
    pub tool_calls: Vec<McpToolCall>,
    pub tool_results: Vec<McpToolResult>,
    pub mcp_tools_fetched: Vec<Json>,
}

#[derive(Debug, Clone)]
pub struct ResponsesMcpFollowupResult {
    pub initial_params: ParameterMap,
    pub follow_up_params: ParameterMap,
    pub call: FollowUpCall,
    pub streaming: Option<McpStreamingPlan>,
    pub events: Vec<StreamEvent>,
    pub response: Option<ResponsesApiResponse>,
}
