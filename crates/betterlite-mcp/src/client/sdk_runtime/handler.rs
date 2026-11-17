pub mod progress;

use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::Arc;

use rmcp::model::{
    ClientConfig, ClientResult, CustomResult, ElicitationAction, ErrorCode, ErrorData,
    Implementation, InitializeRequestParams, JsonObject, ProtocolVersion, ServerNotification,
    ServerRequest,
};
use rmcp::service::{NotificationContext, RequestContext};
use rmcp::{RoleClient, Service};
use serde::{Deserialize, Serialize};

use crate::client::sessions::negotiate::ClientCapabilities as CallbackCapabilities;
use crate::sampling_convert::messages::SamplingMessage;
use crate::sampling_convert::model_preferences::ModelPreferences;
use crate::sampling_convert::response::SamplingResponse;
use crate::sampling_convert::tools::{McpTool, McpToolChoice};

use self::progress::ProgressRegistry;
use super::convert::{DEFAULT_CLIENT_NAME, DEFAULT_CLIENT_VERSION, client_capabilities};

const SAMPLING_UNSUPPORTED: &str = "Sampling not supported";
const ELICITATION_UNSUPPORTED: &str = "Elicitation not supported";
const ROOTS_UNSUPPORTED: &str = "List roots not supported";

// B4744's settings carry no responders yet, so an empty pair keeps every server request on the SDK's default answer
#[derive(Clone, Default)]
pub struct ServerRequestHandlers {
    pub sampling: Option<Arc<dyn SamplingResponder>>,
    pub elicitation: Option<Arc<dyn ElicitationResponder>>,
}

pub trait SamplingResponder: Send + Sync + 'static {
    fn respond<'responder>(
        &'responder self,
        request: SamplingRequest,
    ) -> Pin<Box<dyn Future<Output = Result<SamplingResponse, ErrorData>> + Send + 'responder>>;
}

pub trait ElicitationResponder: Send + Sync + 'static {
    fn respond<'responder>(
        &'responder self,
        request: ElicitationRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ElicitationResult, ErrorData>> + Send + 'responder>>;
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SamplingRequest {
    pub messages: Vec<SamplingMessage>,
    #[serde(default)]
    pub model_preferences: Option<ModelPreferences>,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub include_context: Option<IncludeContext>,
    #[serde(default)]
    pub temperature: Option<f32>,
    pub max_tokens: u32,
    #[serde(default)]
    pub stop_sequences: Option<Vec<String>>,
    #[serde(default)]
    pub metadata: Option<JsonObject>,
    #[serde(default)]
    pub tools: Option<Vec<McpTool>>,
    #[serde(default)]
    pub tool_choice: Option<McpToolChoice>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum IncludeContext {
    None,
    ThisServer,
    AllServers,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum ElicitationRequest {
    #[serde(rename = "form", rename_all = "camelCase")]
    Form {
        message: String,
        requested_schema: JsonObject,
    },
    #[serde(rename = "url", rename_all = "camelCase")]
    Url {
        message: String,
        url: String,
        #[serde(default)]
        elicitation_id: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ElicitationResult {
    pub action: ElicitationAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<JsonObject>,
}

pub struct SdkClientHandler {
    pub(super) handlers: ServerRequestHandlers,
    pub(super) progress: Arc<ProgressRegistry>,
}

impl SdkClientHandler {
    async fn handle_server_request(
        &self,
        request: ServerRequest,
    ) -> Result<ClientResult, ErrorData> {
        // The request arrives as rmcp's own params, which serialize to the wire shape the responders decode
        if let ServerRequest::PingRequest(_) = &request {
            return Ok(ClientResult::empty(()));
        }
        if let ServerRequest::CreateMessageRequest(message) = &request {
            let responder = self
                .handlers
                .sampling
                .as_deref()
                .ok_or_else(|| ErrorData::invalid_request(SAMPLING_UNSUPPORTED, None))?;
            let encoded = serde_json::to_value(&message.params)
                .map_err(|error| internal("sampling", &error))?;
            let params =
                serde_json::from_value(encoded).map_err(|error| invalid("sampling", &error))?;
            return sampling_result(responder.respond(params).await?);
        }
        if let ServerRequest::ElicitRequest(elicitation) = &request {
            let responder = self
                .handlers
                .elicitation
                .as_deref()
                .ok_or_else(|| ErrorData::invalid_request(ELICITATION_UNSUPPORTED, None))?;
            let encoded = serde_json::to_value(&elicitation.params)
                .map_err(|error| internal("elicitation", &error))?;
            let params =
                serde_json::from_value(encoded).map_err(|error| invalid("elicitation", &error))?;
            return custom(&responder.respond(params).await?);
        }
        if let ServerRequest::ListRootsRequest(_) = &request {
            return Err(ErrorData::invalid_request(ROOTS_UNSUPPORTED, None));
        }
        Err(method_not_found(&request))
    }
}

fn sampling_result(response: SamplingResponse) -> Result<ClientResult, ErrorData> {
    match response {
        SamplingResponse::Message(result) => custom(&result),
        SamplingResponse::WithTools(result) => custom(&result),
        SamplingResponse::Error(error) => {
            Err(ErrorData::new(ErrorCode(error.code), error.message, None))
        }
    }
}

fn custom<T>(value: &T) -> Result<ClientResult, ErrorData>
where
    T: Serialize,
{
    serde_json::to_value(value)
        .map_err(|error| internal("result", &error))
        .map(|encoded| ClientResult::CustomResult(CustomResult::new(encoded)))
}

// The Python SDK answers a method it doesn't know with -32601 and the method name as the error data
fn method_not_found(request: &ServerRequest) -> ErrorData {
    let method = serde_json::to_value(request)
        .ok()
        .and_then(|encoded| encoded.get("method").cloned());
    ErrorData::new(ErrorCode::METHOD_NOT_FOUND, "Method not found", method)
}

fn invalid(operation: &str, error: &serde_json::Error) -> ErrorData {
    ErrorData::invalid_params(
        format!("invalid {operation} request"),
        serde_json::to_value(error.to_string()).ok(),
    )
}

// The params and results serialized here are plain structs, so serde cannot fail on them
#[cfg_attr(test, mutants::skip)]
fn internal(operation: &str, error: &serde_json::Error) -> ErrorData {
    ErrorData::internal_error(
        format!("invalid {operation}"),
        serde_json::to_value(error.to_string()).ok(),
    )
}

impl Service<RoleClient> for SdkClientHandler {
    async fn handle_request(
        &self,
        request: ServerRequest,
        _context: RequestContext<RoleClient>,
    ) -> Result<ClientResult, ErrorData> {
        self.handle_server_request(request).await
    }

    fn handle_notification(
        &self,
        notification: ServerNotification,
        _context: NotificationContext<RoleClient>,
    ) -> impl Future<Output = Result<(), ErrorData>> {
        if let ServerNotification::ProgressNotification(progress) = notification {
            self.progress.deliver(progress.params);
        }
        ready(Ok(()))
    }

    fn get_info(&self) -> ClientConfig {
        let callbacks = CallbackCapabilities {
            sampling: self.handlers.sampling.is_some(),
            elicitation: self.handlers.elicitation.is_some(),
        };
        InitializeRequestParams::new(
            client_capabilities(&callbacks),
            Implementation::new(DEFAULT_CLIENT_NAME, DEFAULT_CLIENT_VERSION),
        )
        .with_protocol_version(ProtocolVersion::LATEST_WITH_INITIALIZE)
    }
}
