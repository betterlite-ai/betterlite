#![cfg(test)]

use crate::otel::vocabulary::attribute_keys::{DB, GenAI, HTTP, MCP};
use crate::otel::vocabulary::constants::{
    GenAIOperation, GenAIProvider, LITELLM_PROXY_REQUEST_SPAN_NAME, MCPMethod, NetworkTransport,
};

#[test]
fn enum_values_and_attribute_keys_match_semconv() {
    assert_eq!(GenAIOperation::CHAT.value(), "chat");
    assert_eq!(GenAIProvider::AWS_BEDROCK.value(), "aws.bedrock");
    assert_eq!(MCPMethod::TOOLS_CALL.value(), "tools/call");
    assert_eq!(NetworkTransport::PIPE.value(), "pipe");
    assert_eq!(GenAI::OPERATION_NAME, "gen_ai.operation.name");
    assert_eq!(MCP::METHOD_NAME, "mcp.method.name");
    assert_eq!(DB::SYSTEM_NAME, "db.system.name");
    assert_eq!(HTTP::RESPONSE_STATUS_CODE, "http.response.status_code");
    assert_eq!(
        LITELLM_PROXY_REQUEST_SPAN_NAME,
        "Received Proxy Server Request"
    );
}
