use std::sync::Arc;

use betterlite_prompts::arguments::json::Object;
use rmcp::ServiceError;
use rmcp::model::{
    CallToolResult as SdkCallToolResult, ClientCapabilities, ElicitationCapability, ErrorCode,
    ErrorData, FormElicitationCapability, InputRequiredResult as SdkInputRequiredResult,
    JsonObject, ListToolsResult, ProtocolVersion, SamplingCapability, ServerCapabilities,
    ServerResult, UrlElicitationCapability,
};

use crate::client::cleanup::failure::{ErrorContext, SDK_READ_TIMEOUT_CODE};
use crate::client::sdk_transport::error::SdkError;
use crate::client::sessions::error::{McpFailure, SessionError};
use crate::client::sessions::negotiate::ClientCapabilities as CallbackCapabilities;
use crate::client::tools::result::{CallToolResult, InputRequiredResult, ToolCallResult};
use crate::tools::pagination::Page;

// The MCP SDK's DEFAULT_CLIENT_INFO
pub const DEFAULT_CLIENT_NAME: &str = "mcp";
pub const DEFAULT_CLIENT_VERSION: &str = "0.1.0";
// CONNECTION_CLOSED in the MCP SDK's JSON-RPC error codes
pub const CONNECTION_CLOSED_CODE: i32 = -32000;
pub const CONNECTION_CLOSED_MESSAGE: &str = "Connection closed";
// B4744 tells a timeout apart from a server error reusing the code by this marker in the error data
const TIMEOUT_MARKER: &str = "betterlite/readTimeout";

// Betterlite-core reads the two capabilities LiteLLM consults, so the narrow wire shape is enough here
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerCapabilitiesWire {
    pub prompts: Option<CapabilityWire>,
    pub resources: Option<CapabilityWire>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CapabilityWire {
    pub list_changed: Option<bool>,
}

#[must_use]
pub fn capabilities_wire(capabilities: &ServerCapabilities) -> ServerCapabilitiesWire {
    let prompts = capabilities.prompts.as_ref().map(|prompt| CapabilityWire {
        list_changed: prompt.list_changed,
    });
    let resources = capabilities
        .resources
        .as_ref()
        .map(|resource| CapabilityWire {
            list_changed: resource.list_changed,
        });
    ServerCapabilitiesWire { prompts, resources }
}

pub fn protocol_version(value: &str) -> Result<ProtocolVersion, SdkError> {
    let known = ProtocolVersion::KNOWN_VERSIONS
        .iter()
        .find(|version| version.as_str() == value);
    known.map_or_else(
        || {
            let encoded = serde_json::to_value(value).map_err(SdkError::Json)?;
            serde_json::from_value(encoded).map_err(SdkError::Json)
        },
        |version| Ok(version.clone()),
    )
}

#[must_use]
pub fn client_capabilities(callbacks: &CallbackCapabilities) -> ClientCapabilities {
    let mut capabilities = ClientCapabilities::default();
    if callbacks.sampling {
        capabilities.sampling = Some(SamplingCapability::default());
    }
    if callbacks.elicitation {
        capabilities.elicitation = Some(
            ElicitationCapability::new()
                .with_form(FormElicitationCapability::new())
                .with_url(UrlElicitationCapability::new()),
        );
    }
    capabilities
}

pub fn json_object(object: &Object) -> Result<JsonObject, SdkError> {
    let encoded = serde_json::to_value(object).map_err(SdkError::Json)?;
    let converted = serde_json::from_value(encoded).map_err(SdkError::Json)?;
    Ok(converted)
}

pub fn tool_page(result: &ListToolsResult) -> Result<Page, SdkError> {
    let encoded = serde_json::to_value(result).map_err(SdkError::Json)?;
    let page = serde_json::from_value(encoded).map_err(SdkError::Json)?;
    Ok(page)
}

pub fn tool_call_result(result: &SdkCallToolResult) -> Result<ToolCallResult, SdkError> {
    let encoded = serde_json::to_value(result).map_err(SdkError::Json)?;
    let complete: CallToolResult = serde_json::from_value(encoded).map_err(SdkError::Json)?;
    Ok(ToolCallResult::Complete(complete))
}

pub fn input_required_result(result: &SdkInputRequiredResult) -> Result<ToolCallResult, SdkError> {
    let encoded = serde_json::to_value(result).map_err(SdkError::Json)?;
    let required: InputRequiredResult = serde_json::from_value(encoded).map_err(SdkError::Json)?;
    Ok(ToolCallResult::InputRequired(required))
}

#[must_use]
pub fn unexpected_response(result: &ServerResult) -> SdkError {
    tracing::debug!("unexpected MCP response: {result:?}");
    SdkError::Service(ServiceError::UnexpectedResponse)
}

// LiteLLM's SDK turns a timeout and a closed connection into its own JSON-RPC errors, and keeps every other failure a runtime one
#[must_use]
pub fn request_error(method: &str, error: ServiceError) -> SdkError {
    if let ServiceError::McpError(data) = error {
        return SdkError::JsonRpc(data);
    }
    if let ServiceError::Timeout { .. } = error {
        return SdkError::JsonRpc(ErrorData::new(
            ErrorCode(SDK_READ_TIMEOUT_CODE),
            format!("Request '{method}' timed out"),
            serde_json::to_value(TIMEOUT_MARKER).ok(),
        ));
    }
    if matches!(error, ServiceError::TransportClosed) {
        return SdkError::JsonRpc(ErrorData::new(
            ErrorCode(CONNECTION_CLOSED_CODE),
            CONNECTION_CLOSED_MESSAGE,
            None,
        ));
    }
    SdkError::Service(error)
}

#[must_use]
pub fn protocol_failure(error: &SdkError) -> Option<McpFailure> {
    let SdkError::JsonRpc(data) = error else {
        return None;
    };
    let timed_out = data.code == ErrorCode(SDK_READ_TIMEOUT_CODE)
        && data.data.as_ref().and_then(|value| value.as_str()) == Some(TIMEOUT_MARKER);
    Some(McpFailure {
        code: data.code.0,
        message: data.message.to_string(),
        context: timed_out.then_some(ErrorContext::Timeout),
    })
}

#[must_use]
pub fn session_failure(error: SdkError) -> SessionError<SdkError> {
    protocol_failure(&error).map_or_else(
        || SessionError::Runtime(Arc::new(error)),
        SessionError::Protocol,
    )
}
