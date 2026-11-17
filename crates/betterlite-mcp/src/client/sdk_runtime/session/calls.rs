use std::collections::BTreeMap;
use std::sync::PoisonError;

use rmcp::ServiceError;
use rmcp::model::{
    CallToolRequest, CallToolRequestParams, CallToolResult, ClientRequest, GetPromptRequest,
    GetPromptRequestParams, JsonObject, ListPromptsRequest, ListResourceTemplatesRequest,
    ListResourcesRequest, ListToolsRequest, ListToolsResult, PaginatedRequestParams,
    ProgressNotificationParam, ReadResourceRequest, ReadResourceRequestParams, ServerResult,
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
use super::{ProgressSink, SdkSession, Silent, exchange_with_progress};

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
    let response = exchange_with_progress(session, sdk_request, sink).await?;
    if let ServerResult::InputRequiredResult(result) = &response {
        return input_required_result(result);
    }
    let ServerResult::CallToolResult(result) = &response else {
        return Err(unexpected_response(&response));
    };
    // The SDK validates every non-error tool result against the tool's declared output schema
    if result.is_error != Some(true) {
        validate_tool_result(session, &request.name, result).await?;
    }
    tool_call_result(result)
}

// The schema the SDK validates a tool result against; it lists the tools when it has not seen the tool's schema
async fn cached_output_schema(
    session: &SdkSession,
    name: &str,
) -> Result<Option<JsonObject>, SdkError> {
    let known = session
        .tool_output_schemas
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .contains_key(name);
    if !known {
        let params = PaginatedRequestParams::default();
        let request = ClientRequest::ListToolsRequest(ListToolsRequest::with_param(params));
        let response = exchange_with_progress(session, request, &Silent).await?;
        let ServerResult::ListToolsResult(listing) = &response else {
            return Err(unexpected_response(&response));
        };
        session.absorb_tool_listing(None, listing);
    }
    let cached = session
        .tool_output_schemas
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(name)
        .cloned()
        .flatten();
    Ok(cached)
}

// The SDK insists on structured content that matches the tool's declared output schema
async fn validate_tool_result(
    session: &SdkSession,
    name: &str,
    result: &CallToolResult,
) -> Result<(), SdkError> {
    let Some(output_schema) = cached_output_schema(session, name).await? else {
        return Ok(());
    };
    let Some(content) = result.structured_content.as_ref() else {
        return Err(SdkError::Service(ServiceError::UnexpectedResponse));
    };
    let encoded = serde_json::to_value(&output_schema).map_err(SdkError::Json)?;
    let validator = jsonschema::options()
        .should_validate_formats(false)
        .offline()
        .build(&encoded)
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
        let response = exchange_with_progress(self, request, &Silent)
            .await
            .map_err(session_failure)?;
        let ServerResult::ListToolsResult(result) = &response else {
            return Err(session_failure(unexpected_response(&response)));
        };
        self.absorb_tool_listing(cursor, result);
        tool_page(result).map_err(session_failure)
    }

    async fn call_tool(
        &mut self,
        request: CallRequest,
    ) -> Result<ToolCallResult, SessionError<SdkError>> {
        let result = call_tool_request(self, &request, &Silent)
            .await
            .map_err(session_failure)?;
        if let ToolCallResult::InputRequired(_) = result {
            return Err(session_failure(SdkError::Service(
                ServiceError::UnexpectedResponse,
            )));
        }
        Ok(result)
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
        if allow_input_required {
            return Ok(result);
        }
        if let ToolCallResult::InputRequired(_) = result {
            return Err(session_failure(SdkError::Service(
                ServiceError::UnexpectedResponse,
            )));
        }
        Ok(result)
    }
}

fn prompt_arguments(arguments: &BTreeMap<String, String>) -> JsonObject {
    arguments
        .iter()
        .map(|(key, value)| (key.clone(), serde_json::to_value(value).unwrap_or_default()))
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
        let response = exchange_with_progress(self, request, &Silent).await?;
        if let ServerResult::ListPromptsResult(result) = &response {
            return serde_json::to_string(result).map_err(SdkError::Json);
        }
        if let ServerResult::ListResourcesResult(result) = &response {
            return serde_json::to_string(result).map_err(SdkError::Json);
        }
        if let ServerResult::ListResourceTemplatesResult(result) = &response {
            return serde_json::to_string(result).map_err(SdkError::Json);
        }
        if let ServerResult::ReadResourceResult(result) = &response {
            return serde_json::to_string(result).map_err(SdkError::Json);
        }
        Err(unexpected_response(&response))
    }

    pub async fn get_prompt_wire(
        &self,
        name: &str,
        arguments: Option<&BTreeMap<String, String>>,
    ) -> Result<String, SdkError> {
        let mut params = GetPromptRequestParams::new(name.to_owned());
        if let Some(values) = arguments {
            params = params.with_arguments(prompt_arguments(values));
        }
        let request = ClientRequest::GetPromptRequest(GetPromptRequest::new(params));
        let response = exchange_with_progress(self, request, &Silent).await?;
        let ServerResult::GetPromptResult(result) = &response else {
            return Err(unexpected_response(&response));
        };
        serde_json::to_string(result).map_err(SdkError::Json)
    }

    // The SDK caches each listing's output schemas and drops the rest only when the request carried no cursor and no page follows
    fn absorb_tool_listing(&self, cursor: Option<&str>, listing: &ListToolsResult) {
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
        if cursor.is_none() && listing.next_cursor.is_none() {
            schemas.retain(|name, _| {
                listing
                    .tools
                    .iter()
                    .any(|tool| tool.name.as_ref() == name.as_str())
            });
        }
    }
}
