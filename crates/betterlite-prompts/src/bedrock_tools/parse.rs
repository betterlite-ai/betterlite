use crate::arguments::json::Json;
use crate::bedrock_tools::capabilities::BedrockToolCapabilities;
use crate::bedrock_tools::error::BedrockToolError;
use crate::bedrock_tools::tools::{BedrockTools, bedrock_tools};

pub fn tools_from_json_bytes(
    bytes: &[u8],
    capabilities: BedrockToolCapabilities,
) -> Result<BedrockTools, BedrockToolError> {
    let tools: Vec<Json> =
        serde_json::from_slice(bytes).map_err(|source| BedrockToolError::Json { source })?;
    bedrock_tools(&tools, capabilities)
}
