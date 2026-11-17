use std::borrow::Cow;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorData,
    Implementation, JsonObject, ListToolsResult, PaginatedRequestParams, ProgressNotificationParam,
    ProtocolVersion, ServerCapabilities, ServerConfig, ServerRequest, Tool,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{RoleServer, ServerHandler, ServiceError};
use serde_json::json;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::sleep;

use crate::client::sdk_runtime::handler::{
    ElicitationRequest, ElicitationResponder, ElicitationResult, SamplingRequest, SamplingResponder,
};
use crate::sampling_convert::content::{McpContentBlock, SamplingRole};
use crate::sampling_convert::response::{CreateMessageResult, SamplingResponse, StopReason};

use super::support;

pub(crate) const INSTRUCTIONS: &str = "the test server's instructions";

// One handler drives every transport test: tools pick the behaviour, so a case reads from the tool name alone
#[derive(Clone)]
pub(crate) struct TestServer {
    protocol_version: ProtocolVersion,
    supported: Vec<ProtocolVersion>,
    instructions: &'static str,
}

impl TestServer {
    pub(crate) fn standard() -> Self {
        Self {
            protocol_version: ProtocolVersion::LATEST_WITH_INITIALIZE,
            supported: ProtocolVersion::KNOWN_VERSIONS.to_vec(),
            instructions: INSTRUCTIONS,
        }
    }

    pub(crate) fn legacy() -> Self {
        Self {
            protocol_version: ProtocolVersion::V_2025_06_18,
            supported: vec![ProtocolVersion::V_2025_06_18],
            instructions: INSTRUCTIONS,
        }
    }

    fn config(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_prompts()
                .enable_resources()
                .enable_prompts_list_changed()
                .build(),
        )
        .with_server_info(Implementation::new("betterlite-test", "1.0"))
        .with_protocol_version(self.protocol_version.clone())
        .with_instructions(self.instructions)
    }
}

impl ServerHandler for TestServer {
    fn get_info(&self) -> ServerConfig {
        self.config()
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Owned(self.supported.clone())
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let cursor = request.and_then(|params| params.cursor);
        let (tools, next) = match cursor.as_deref() {
            None => (
                vec![
                    Tool::new("alpha", "the first tool", JsonObject::new()),
                    Tool::new("beta", "the second tool", JsonObject::new()),
                ],
                Some("page-2".to_owned()),
            ),
            Some("page-2") => (
                vec![Tool::new("gamma", "the third tool", JsonObject::new())],
                None,
            ),
            Some(_) => (Vec::new(), None),
        };
        let mut result = ListToolsResult::with_all_items(tools);
        result.next_cursor = next;
        Ok(result)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let outcome = match request.name.as_str() {
            "echo" => CallToolResult::success(vec![ContentBlock::text("echoed")]),
            "fail" => CallToolResult::error(vec![ContentBlock::text("the tool failed")]),
            "rpc-error" => return Err(ErrorData::invalid_params("bad arguments", None)),
            "progress" => {
                let token = context
                    .meta
                    .get_progress_token()
                    .ok_or_else(|| ErrorData::invalid_params("missing progress token", None))?;
                for step in [25.0_f64, 75.0_f64] {
                    context
                        .peer
                        .notify_progress(
                            ProgressNotificationParam::new(token.clone(), step)
                                .with_total(100.0)
                                .with_message("working"),
                        )
                        .await
                        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
                }
                CallToolResult::success(vec![ContentBlock::text("progressed")])
            }
            "slow" => {
                sleep(support::CALL_TIMEOUT.saturating_mul(6)).await;
                CallToolResult::success(vec![ContentBlock::text("too late")])
            }
            "sample" => {
                let request: ServerRequest = serde_json::from_value(json!({
                    "method": "sampling/createMessage",
                    "params": {
                        "messages": [{"role": "user", "content": {"type": "text", "text": "hello"}}],
                        "maxTokens": 16
                    }
                }))
                .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
                CallToolResult::success(vec![ContentBlock::text(
                    peer_answer(&context, request).await,
                )])
            }
            "elicit" => {
                let request: ServerRequest = serde_json::from_value(json!({
                    "method": "elicitation/create",
                    "params": {
                        "mode": "form",
                        "message": "who are you?",
                        "requestedSchema": {"type": "object", "properties": {}}
                    }
                }))
                .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
                CallToolResult::success(vec![ContentBlock::text(
                    peer_answer(&context, request).await,
                )])
            }
            "roots" => {
                let request: ServerRequest =
                    serde_json::from_value(json!({"method": "roots/list"}))
                        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
                CallToolResult::success(vec![ContentBlock::text(
                    peer_answer(&context, request).await,
                )])
            }
            "custom" => {
                let request: ServerRequest = serde_json::from_value(json!({
                    "method": "not/a-real-method",
                    "params": {}
                }))
                .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
                CallToolResult::success(vec![ContentBlock::text(
                    peer_answer(&context, request).await,
                )])
            }
            other => {
                return Err(ErrorData::invalid_params(
                    format!("unknown tool {other}"),
                    None,
                ));
            }
        };
        Ok(CallToolResponse::from(outcome))
    }
}

// The outcome of a server-initiated request is reported through the tool result, so a case asserts on the wire answer
async fn peer_answer(context: &RequestContext<RoleServer>, request: ServerRequest) -> String {
    match context.peer.send_request(request).await {
        Ok(result) => serde_json::to_string(&result).unwrap_or_default(),
        Err(ServiceError::McpError(error)) => match &error.data {
            Some(data) => format!("error {}: {}: {data}", error.code.0, error.message),
            None => format!("error {}: {}", error.code.0, error.message),
        },
        Err(error) => format!("failure: {error}"),
    }
}

pub(crate) struct HttpServer {
    pub(crate) url: String,
    _tasks: JoinSet<()>,
}

pub(crate) async fn serve_http(server: TestServer) -> HttpServer {
    let config = StreamableHttpServerConfig::default();
    let service: StreamableHttpService<TestServer, LocalSessionManager> =
        StreamableHttpService::new(move || Ok(server.clone()), Default::default(), config);
    let router = axum::Router::new().nest_service("/mcp", service);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address: SocketAddr = listener.local_addr().unwrap();
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        drop(axum::serve(listener, router).await);
    });
    HttpServer {
        url: format!("http://{address}/mcp"),
        _tasks: tasks,
    }
}

pub(crate) struct TestSampling;

impl SamplingResponder for TestSampling {
    fn respond<'a>(
        &'a self,
        request: SamplingRequest,
    ) -> Pin<Box<dyn Future<Output = Result<SamplingResponse, ErrorData>> + Send + 'a>> {
        Box::pin(async move { Ok(sampled(&request)) })
    }
}

fn sampled(request: &SamplingRequest) -> SamplingResponse {
    let text = format!("{} message(s)", request.messages.len());
    SamplingResponse::Message(CreateMessageResult {
        role: SamplingRole::Assistant,
        content: McpContentBlock::Text { text },
        model: "test-model".to_owned(),
        stop_reason: StopReason::EndTurn,
    })
}

pub(crate) struct TestElicitation;

impl ElicitationResponder for TestElicitation {
    fn respond<'a>(
        &'a self,
        request: ElicitationRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ElicitationResult, ErrorData>> + Send + 'a>> {
        Box::pin(async move { Ok(accepted(&request)) })
    }
}

fn accepted(request: &ElicitationRequest) -> ElicitationResult {
    let message = match request {
        ElicitationRequest::Form { message, .. } | ElicitationRequest::Url { message, .. } => {
            message
        }
    };
    let mut content = JsonObject::new();
    content.insert("answer".to_owned(), json!(message));
    ElicitationResult {
        action: rmcp::model::ElicitationAction::Accept,
        content: Some(content),
    }
}
