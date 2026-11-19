use betterlite_openai::search::credentials::CredentialError;
use thiserror::Error;

use crate::signing::error::SigningError;

#[derive(Debug, Error)]
pub enum SearchError {
    #[error(
        "AGENTCORE_GATEWAY_URL is not set. Set it to your AgentCore Gateway MCP endpoint \
         (https://<gateway-id>.gateway.bedrock-agentcore.<region>.amazonaws.com/mcp) or pass \
         api_base."
    )]
    GatewayUrlMissing,
    #[error(
        "Invalid AgentCore search tool_name '{tool_name}': must end with '___WebSearch' (a \
         web-search connector tool). Other gateway tools cannot be invoked through this provider."
    )]
    ToolName { tool_name: String },
    #[error("AgentCore search expects a single dict request body")]
    RequestBody,
    #[error(
        "Refusing to send AgentCore credentials over plaintext HTTP to '{api_base}': a bearer \
         token or SigV4 signature would be readable in transit. Use an https gateway URL (plain \
         http is allowed only for localhost)."
    )]
    PlaintextTransport { api_base: String },
    #[error(
        "Refusing to send SigV4-signed AgentCore requests to '{api_base}': it is neither an \
         AgentCore gateway hostname nor the host in AGENTCORE_GATEWAY_URL. Set \
         AGENTCORE_GATEWAY_URL to authorize a custom gateway hostname."
    )]
    UntrustedSigningHost { api_base: String },
    #[error(
        "Cannot derive the SigV4 signing region from api_base '{api_base}' or the AWS \
         configuration chain. Set aws_region_name (or AWS_DEFAULT_REGION / a profile region) to \
         the gateway's region when using a custom hostname."
    )]
    SigningRegion { api_base: String },
    #[error("{field} must be a string")]
    WrongType { field: &'static str },
    #[error("the request parameters cannot be encoded")]
    Parameters(#[source] serde_json::Error),
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error(transparent)]
    Signing(#[from] SigningError),
}
