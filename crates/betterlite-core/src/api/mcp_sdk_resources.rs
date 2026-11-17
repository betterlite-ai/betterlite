use betterlite_mcp::client::sdk_runtime::convert::protocol_failure as sdk_protocol_failure;
use betterlite_mcp::client::sdk_runtime::session::SdkSession;
use betterlite_mcp::client::sdk_runtime::session::calls::WireRequest;
use betterlite_mcp::client::sdk_transport::error::SdkError;
use betterlite_mcp::client::sessions::error::McpFailure;
use url::Url;

use super::mcp_client_resources::session::ResourceSession;
use super::mcp_client_resources::types::{
    Capability, GetPromptRequestParams, GetPromptResult, PromptPage, ReadResourceResult,
    ResourcePage, ResourceTemplatePage, ServerCapabilities,
};

#[path = "mcp_sdk_resources/tests.rs"]
mod tests;

// The SDK session decodes every prompt and resource result through the MCP wire shape they share with these types
impl ResourceSession for SdkSession {
    type Error = SdkError;

    fn server_capabilities(&self) -> Option<ServerCapabilities> {
        let wired = self.server_capabilities.as_ref()?;
        Some(ServerCapabilities {
            prompts: wired.prompts.map(|capability| Capability {
                list_changed: capability.list_changed,
            }),
            resources: wired.resources.map(|capability| Capability {
                list_changed: capability.list_changed,
            }),
        })
    }

    async fn list_prompts(&mut self, cursor: Option<String>) -> Result<PromptPage, SdkError> {
        let wire = self
            .wire_request(WireRequest::ListPrompts(cursor.as_deref()))
            .await?;
        let page = serde_json::from_str(&wire).map_err(SdkError::Json)?;
        Ok(page)
    }

    async fn get_prompt(
        &mut self,
        params: GetPromptRequestParams,
    ) -> Result<GetPromptResult, SdkError> {
        let wire = self
            .get_prompt_wire(&params.name, params.arguments.as_ref())
            .await?;
        let result = serde_json::from_str(&wire).map_err(SdkError::Json)?;
        Ok(result)
    }

    async fn list_resources(&mut self, cursor: Option<String>) -> Result<ResourcePage, SdkError> {
        let wire = self
            .wire_request(WireRequest::ListResources(cursor.as_deref()))
            .await?;
        let page = serde_json::from_str(&wire).map_err(SdkError::Json)?;
        Ok(page)
    }

    async fn list_resource_templates(
        &mut self,
        cursor: Option<String>,
    ) -> Result<ResourceTemplatePage, SdkError> {
        let wire = self
            .wire_request(WireRequest::ListResourceTemplates(cursor.as_deref()))
            .await?;
        let page = serde_json::from_str(&wire).map_err(SdkError::Json)?;
        Ok(page)
    }

    async fn read_resource(&mut self, url: Url) -> Result<ReadResourceResult, SdkError> {
        let wire = self
            .wire_request(WireRequest::ReadResource(url.as_str()))
            .await?;
        let result = serde_json::from_str(&wire).map_err(SdkError::Json)?;
        Ok(result)
    }

    fn protocol_failure(error: &SdkError) -> Option<McpFailure> {
        sdk_protocol_failure(error)
    }
}
