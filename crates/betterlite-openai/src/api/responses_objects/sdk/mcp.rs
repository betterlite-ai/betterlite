use betterlite_models::decisions::Json;

record! { McpCall: McpCallTag = "mcp_call" {
    id: String,
    arguments: String,
    name: String,
    server_label: String,
    approval_request_id: Option<String>,
    error: Option<String>,
    output: Option<String>,
    status: Option<super::literals::McpCallStatusChoice>,
} }
record! { McpListTools: McpListToolsTag = "mcp_list_tools" {
    id: String,
    server_label: String,
    tools: Vec<McpTool>,
    error: Option<String>,
} }
record! { McpTool {
    #[serde(deserialize_with = "Json::deserialize")]
    input_schema: Json,
    name: String,
    annotations: Option<Json>,
    description: Option<String>,
} }
record! { McpApprovalRequest: McpApprovalRequestTag = "mcp_approval_request" {
    id: String,
    arguments: String,
    name: String,
    server_label: String,
} }
record! { McpApprovalResponse: McpApprovalResponseTag = "mcp_approval_response" {
    id: String,
    approval_request_id: String,
    approve: bool,
    reason: Option<String>,
} }
