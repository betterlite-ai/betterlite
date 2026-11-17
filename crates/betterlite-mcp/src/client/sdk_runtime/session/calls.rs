use std::collections::BTreeMap;

use rmcp::ServiceError;
use rmcp::model::{
    CallToolRequest, CallToolRequestParams, ClientRequest, GetPromptRequest,
    GetPromptRequestParams, JsonObject, ListPromptsRequest, ListResourceTemplatesRequest,
    ListResourcesRequest, ListToolsRequest, PaginatedRequestParams, ProgressNotificationParam,
    ReadResourceRequest, ReadResourceRequestParams, ServerResult,
};

use crate::client::sdk_transport::error::SdkError;
use crate::client::sessions::error::SessionError;
use crate::client::tools::progress::{ProgressForwarder, ProgressHost};
use crate::client::tools::result::ToolCallResult;
use crate::client::tools::session::ToolCallSession;
use crate::tools::pagination::{McpSession, Page};
use crate::tools::tool_calls::CallRequest;

use super::super::convert::{
    input_required_result, json_object, session_failure, tool_call_result, tool_page,
    unexpected_response,
};
use super::{ProgressSink, SdkSession, Silent};

struct Forward<'forward, 'host, Host>
where
    Host: ProgressHost + Sync,
{
    forwarder: &'forward ProgressForwarder<'host, Host>,
}

impl<Host> ProgressSink for Forward<'_, '_, Host>
where
    Host: ProgressHost + Sync,
{
    async fn send(&self, param: ProgressNotificationParam) {
        self.forwarder
            .report(param.progress, param.total, param.message.as_deref())
            .await;
    }
}

async fn call_tool_request<Sink>(
    session: &SdkSession,
    request: &CallRequest,
    sink: &Sink,
) -> Result<ToolCallResult, SdkError>
where
    Sink: ProgressSink,
{
    let mut params = CallToolRequestParams::new(request.name.clone());
    if let Some(arguments) = request.arguments.as_ref() {
        params = params.with_arguments(json_object(arguments)?);
    }
    let sdk_request = ClientRequest::CallToolRequest(CallToolRequest::new(params));
    match session.exchange_with_progress(sdk_request, sink).await? {
        ServerResult::CallToolResult(result) => tool_call_result(&result),
        ServerResult::InputRequiredResult(result) => input_required_result(&result),
        other => Err(unexpected_response(&other)),
    }
}

impl McpSession for SdkSession {
    type Error = SessionError<SdkError>;
    type Output = ToolCallResult;

    async fn list_tools(&mut self, cursor: Option<&str>) -> Result<Page, SessionError<SdkError>> {
        let params = PaginatedRequestParams::default().with_cursor(cursor.map(str::to_owned));
        let request = ClientRequest::ListToolsRequest(ListToolsRequest::with_param(params));
        let response = self
            .exchange_with_progress(request, &Silent)
            .await
            .map_err(session_failure)?;
        match response {
            ServerResult::ListToolsResult(result) => tool_page(&result).map_err(session_failure),
            other => Err(session_failure(unexpected_response(&other))),
        }
    }

    async fn call_tool(
        &mut self,
        request: CallRequest,
    ) -> Result<ToolCallResult, SessionError<SdkError>> {
        let result = call_tool_request(self, &request, &Silent)
            .await
            .map_err(session_failure)?;
        match result {
            ToolCallResult::InputRequired(_) => Err(session_failure(SdkError::Service(
                ServiceError::UnexpectedResponse,
            ))),
            result => Ok(result),
        }
    }
}

impl ToolCallSession for SdkSession {
    type Error = SessionError<SdkError>;

    async fn call_tool<Host>(
        &mut self,
        request: &CallRequest,
        progress: &ProgressForwarder<'_, Host>,
        allow_input_required: bool,
    ) -> Result<ToolCallResult, Self::Error>
    where
        Host: ProgressHost + Sync,
    {
        let sink = Forward {
            forwarder: progress,
        };
        let result = call_tool_request(self, request, &sink)
            .await
            .map_err(session_failure)?;
        match (result, allow_input_required) {
            (ToolCallResult::InputRequired(_), false) => Err(session_failure(SdkError::Service(
                ServiceError::UnexpectedResponse,
            ))),
            (result, _) => Ok(result),
        }
    }
}

fn prompt_arguments(arguments: &BTreeMap<String, String>) -> JsonObject {
    arguments
        .iter()
        .map(|(key, value)| (key.clone(), serde_json::json!(value)))
        .collect()
}

// The prompt and resource results belong to betterlite-core's types, so the session hands their wire text on
impl SdkSession {
    pub async fn list_prompts_wire(&self, cursor: Option<&str>) -> Result<String, SdkError> {
        let params = PaginatedRequestParams::default().with_cursor(cursor.map(str::to_owned));
        let request = ClientRequest::ListPromptsRequest(ListPromptsRequest::with_param(params));
        match self.exchange_with_progress(request, &Silent).await? {
            ServerResult::ListPromptsResult(result) => {
                serde_json::to_string(&result).map_err(SdkError::Json)
            }
            other => Err(unexpected_response(&other)),
        }
    }

    pub async fn get_prompt_wire(
        &self,
        name: &str,
        arguments: Option<&BTreeMap<String, String>>,
    ) -> Result<String, SdkError> {
        let mut params = GetPromptRequestParams::new(name.to_owned());
        if let Some(arguments) = arguments {
            params = params.with_arguments(prompt_arguments(arguments));
        }
        let request = ClientRequest::GetPromptRequest(GetPromptRequest::new(params));
        match self.exchange_with_progress(request, &Silent).await? {
            ServerResult::GetPromptResult(result) => {
                serde_json::to_string(&result).map_err(SdkError::Json)
            }
            other => Err(unexpected_response(&other)),
        }
    }

    pub async fn list_resources_wire(&self, cursor: Option<&str>) -> Result<String, SdkError> {
        let params = PaginatedRequestParams::default().with_cursor(cursor.map(str::to_owned));
        let request = ClientRequest::ListResourcesRequest(ListResourcesRequest::with_param(params));
        match self.exchange_with_progress(request, &Silent).await? {
            ServerResult::ListResourcesResult(result) => {
                serde_json::to_string(&result).map_err(SdkError::Json)
            }
            other => Err(unexpected_response(&other)),
        }
    }

    pub async fn list_resource_templates_wire(
        &self,
        cursor: Option<&str>,
    ) -> Result<String, SdkError> {
        let params = PaginatedRequestParams::default().with_cursor(cursor.map(str::to_owned));
        let request = ClientRequest::ListResourceTemplatesRequest(
            ListResourceTemplatesRequest::with_param(params),
        );
        match self.exchange_with_progress(request, &Silent).await? {
            ServerResult::ListResourceTemplatesResult(result) => {
                serde_json::to_string(&result).map_err(SdkError::Json)
            }
            other => Err(unexpected_response(&other)),
        }
    }

    pub async fn read_resource_wire(&self, uri: &str) -> Result<String, SdkError> {
        let params = ReadResourceRequestParams::new(uri.to_owned());
        let request = ClientRequest::ReadResourceRequest(ReadResourceRequest::new(params));
        match self.exchange_with_progress(request, &Silent).await? {
            ServerResult::ReadResourceResult(result) => {
                serde_json::to_string(&result).map_err(SdkError::Json)
            }
            other => Err(unexpected_response(&other)),
        }
    }
}
