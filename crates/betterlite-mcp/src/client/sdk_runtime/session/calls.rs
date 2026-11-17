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
use crate::tools::pagination::{McpSession, Page};
use crate::tools::progress::{ProgressForwarder, ProgressHost};
use crate::tools::result::ToolCallResult;
use crate::tools::session::ToolCallSession;
use crate::tools::tool_calls::CallRequest;

use super::super::convert::{
    input_required_result, json_object, session_failure, tool_call_result, tool_page,
    unexpected_response,
};
use super::{ProgressSink, SdkSession, Silent};

struct Forward<'a, 'host, Host>
where
    Host: ProgressHost + Sync,
{
    forwarder: &'a ProgressForwarder<'host, Host>,
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
    let request = ClientRequest::CallToolRequest(CallToolRequest::new(params));
    match session.exchange_with_progress(request, sink).await? {
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
        match self.exchange(request).await.map_err(session_failure)? {
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

fn paginated(cursor: Option<&str>) -> PaginatedRequestParams {
    PaginatedRequestParams::default().with_cursor(cursor.map(str::to_owned))
}

fn wire<T>(value: &T) -> Result<String, SdkError>
where
    T: serde::Serialize,
{
    serde_json::to_string(value).map_err(SdkError::Json)
}

fn prompt_arguments(arguments: &BTreeMap<String, String>) -> JsonObject {
    arguments
        .iter()
        .map(|(key, value)| (key.clone(), serde_json::json!(value)))
        .collect()
}

fn prompts_request(cursor: Option<&str>) -> ClientRequest {
    ClientRequest::ListPromptsRequest(ListPromptsRequest::with_param(paginated(cursor)))
}

fn resources_request(cursor: Option<&str>) -> ClientRequest {
    ClientRequest::ListResourcesRequest(ListResourcesRequest::with_param(paginated(cursor)))
}

fn templates_request(cursor: Option<&str>) -> ClientRequest {
    let request = ListResourceTemplatesRequest::with_param(paginated(cursor));
    ClientRequest::ListResourceTemplatesRequest(request)
}

fn prompt_request(
    name: &str,
    arguments: Option<&BTreeMap<String, String>>,
) -> Result<ClientRequest, SdkError> {
    let mut params = GetPromptRequestParams::new(name.to_owned());
    if let Some(arguments) = arguments {
        params = params.with_arguments(prompt_arguments(arguments));
    }
    Ok(ClientRequest::GetPromptRequest(GetPromptRequest::new(
        params,
    )))
}

fn read_request(uri: &str) -> ClientRequest {
    ClientRequest::ReadResourceRequest(ReadResourceRequest::new(ReadResourceRequestParams::new(
        uri.to_owned(),
    )))
}

fn prompts_wire(response: &ServerResult) -> Result<String, SdkError> {
    match response {
        ServerResult::ListPromptsResult(result) => wire(result),
        other => Err(unexpected_response(other)),
    }
}

fn prompt_wire(response: &ServerResult) -> Result<String, SdkError> {
    match response {
        ServerResult::GetPromptResult(result) => wire(result),
        other => Err(unexpected_response(other)),
    }
}

fn resources_wire(response: &ServerResult) -> Result<String, SdkError> {
    match response {
        ServerResult::ListResourcesResult(result) => wire(result),
        other => Err(unexpected_response(other)),
    }
}

fn templates_wire(response: &ServerResult) -> Result<String, SdkError> {
    match response {
        ServerResult::ListResourceTemplatesResult(result) => wire(result),
        other => Err(unexpected_response(other)),
    }
}

fn resource_wire(response: &ServerResult) -> Result<String, SdkError> {
    match response {
        ServerResult::ReadResourceResult(result) => wire(result),
        other => Err(unexpected_response(other)),
    }
}

// The prompt and resource results belong to betterlite-core's types, so the session hands their wire text on
impl SdkSession {
    async fn wire_response(
        &self,
        request: ClientRequest,
        pick: fn(&ServerResult) -> Result<String, SdkError>,
    ) -> Result<String, SdkError> {
        pick(&self.exchange(request).await?)
    }

    pub async fn list_prompts_wire(&self, cursor: Option<&str>) -> Result<String, SdkError> {
        self.wire_response(prompts_request(cursor), prompts_wire)
            .await
    }

    pub async fn get_prompt_wire(
        &self,
        name: &str,
        arguments: Option<&BTreeMap<String, String>>,
    ) -> Result<String, SdkError> {
        self.wire_response(prompt_request(name, arguments)?, prompt_wire)
            .await
    }

    pub async fn list_resources_wire(&self, cursor: Option<&str>) -> Result<String, SdkError> {
        self.wire_response(resources_request(cursor), resources_wire)
            .await
    }

    pub async fn list_resource_templates_wire(
        &self,
        cursor: Option<&str>,
    ) -> Result<String, SdkError> {
        self.wire_response(templates_request(cursor), templates_wire)
            .await
    }

    pub async fn read_resource_wire(&self, uri: &str) -> Result<String, SdkError> {
        self.wire_response(read_request(uri), resource_wire).await
    }
}
