use std::error::Error;
use std::future::Future;
use std::sync::Arc;

use betterlite_mcp::client::sessions::error::{McpFailure, SessionError};
use betterlite_mcp::client::sessions::runtime::{McpRuntime, SessionContext};

use super::types::{
    GetPromptRequestParams, GetPromptResult, PromptPage, ReadResourceResult, ResourcePage,
    ResourceTemplatePage, ServerCapabilities,
};

pub type RuntimeSession<Runtime> = <<Runtime as McpRuntime>::Session as SessionContext>::Session;

pub trait ResourceSession {
    type Error: Error + Send + Sync + 'static;

    fn server_capabilities(&self) -> Option<ServerCapabilities>;

    fn list_prompts(
        &mut self,
        cursor: Option<String>,
    ) -> impl Future<Output = Result<PromptPage, Self::Error>> + Send;

    fn get_prompt(
        &mut self,
        params: GetPromptRequestParams,
    ) -> impl Future<Output = Result<GetPromptResult, Self::Error>> + Send;

    fn list_resources(
        &mut self,
        cursor: Option<String>,
    ) -> impl Future<Output = Result<ResourcePage, Self::Error>> + Send;

    fn list_resource_templates(
        &mut self,
        cursor: Option<String>,
    ) -> impl Future<Output = Result<ResourceTemplatePage, Self::Error>> + Send;

    fn read_resource(
        &mut self,
        uri: String,
    ) -> impl Future<Output = Result<ReadResourceResult, Self::Error>> + Send;

    fn protocol_failure(error: &Self::Error) -> Option<McpFailure>;
}

pub(crate) fn session_error<Session>(
    _session: &Session,
    error: Session::Error,
) -> SessionError<Session::Error>
where
    Session: ResourceSession,
{
    Session::protocol_failure(&error).map_or_else(
        || SessionError::Runtime(Arc::new(error)),
        SessionError::Protocol,
    )
}
