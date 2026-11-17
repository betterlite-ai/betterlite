use std::collections::BTreeMap;
use std::sync::PoisonError;

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
        ServerResult::CallToolResult(result) => {
            // The SDK validates every non-error tool result against the tool's declared output schema
            if result.is_error != Some(true) {
                validate_tool_result(session, &request.name, &result).await?;
            }
            tool_call_result(&result)
        }
        ServerResult::InputRequiredResult(result) => input_required_result(&result),
        other => Err(unexpected_response(&other)),
    }
}

// The SDK lists the tools when it has not seen the tool's schema, then insists on structured content that matches it
async fn validate_tool_result(
    session: &SdkSession,
    name: &str,
    result: &rmcp::model::CallToolResult,
) -> Result<(), SdkError> {
    let known = session
        .tool_output_schemas
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .contains_key(name);
    if !known {
        let params = PaginatedRequestParams::default();
        let request = ClientRequest::ListToolsRequest(ListToolsRequest::with_param(params));
        let response = session.exchange_with_progress(request, &Silent).await?;
        let ServerResult::ListToolsResult(listing) = response else {
            return Err(unexpected_response(&response));
        };
        session.absorb_tool_listing(&listing);
    }
    let schema = session
        .tool_output_schemas
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(name)
        .cloned()
        .flatten();
    let Some(schema) = schema else {
        return Ok(());
    };
    let Some(content) = result.structured_content.as_ref() else {
        return Err(SdkError::Service(ServiceError::UnexpectedResponse));
    };
    let schema = serde_json::Value::Object(schema);
    let validator = jsonschema::options()
        .should_validate_formats(false)
        .offline()
        .build(&schema)
        .map_err(|_error| SdkError::Service(ServiceError::UnexpectedResponse))?;
    validator
        .validate(content)
        .map_err(|_error| SdkError::Service(ServiceError::UnexpectedResponse))
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
            ServerResult::ListToolsResult(result) => {
                self.absorb_tool_listing(&result);
                tool_page(&result).map_err(session_failure)
            }
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

// Which prompt or resource request the session sends; the result comes back as wire text for betterlite-core's types
pub enum WireRequest<'request> {
    ListPrompts(Option<&'request str>),
    ListResources(Option<&'request str>),
    ListResourceTemplates(Option<&'request str>),
    ReadResource(&'request str),
}

impl SdkSession {
    pub async fn wire_request(&self, wire: WireRequest<'_>) -> Result<String, SdkError> {
        let request = match wire {
            WireRequest::ListPrompts(cursor) => {
                let params =
                    PaginatedRequestParams::default().with_cursor(cursor.map(str::to_owned));
                ClientRequest::ListPromptsRequest(ListPromptsRequest::with_param(params))
            }
            WireRequest::ListResources(cursor) => {
                let params =
                    PaginatedRequestParams::default().with_cursor(cursor.map(str::to_owned));
                ClientRequest::ListResourcesRequest(ListResourcesRequest::with_param(params))
            }
            WireRequest::ListResourceTemplates(cursor) => {
                let params =
                    PaginatedRequestParams::default().with_cursor(cursor.map(str::to_owned));
                ClientRequest::ListResourceTemplatesRequest(
                    ListResourceTemplatesRequest::with_param(params),
                )
            }
            WireRequest::ReadResource(uri) => ClientRequest::ReadResourceRequest(
                ReadResourceRequest::new(ReadResourceRequestParams::new(uri.to_owned())),
            ),
        };
        match self.exchange_with_progress(request, &Silent).await? {
            ServerResult::ListPromptsResult(result) => {
                serde_json::to_string(&result).map_err(SdkError::Json)
            }
            ServerResult::ListResourcesResult(result) => {
                serde_json::to_string(&result).map_err(SdkError::Json)
            }
            ServerResult::ListResourceTemplatesResult(result) => {
                serde_json::to_string(&result).map_err(SdkError::Json)
            }
            ServerResult::ReadResourceResult(result) => {
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

    // The SDK caches each listing's output schemas and drops the ones a complete listing no longer carries
    fn absorb_tool_listing(&self, listing: &rmcp::model::ListToolsResult) {
        let mut schemas = self
            .tool_output_schemas
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for tool in &listing.tools {
            schemas.insert(
                tool.name.to_string(),
                tool.output_schema.as_deref().cloned(),
            );
        }
        if listing.next_cursor.is_none() {
            schemas.retain(|name, _| {
                listing
                    .tools
                    .iter()
                    .any(|tool| tool.name.as_ref() == name.as_str())
            });
        }
    }
}
