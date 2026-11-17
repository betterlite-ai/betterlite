use std::borrow::Cow;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorData,
    Implementation, JsonObject, ListToolsResult, PaginatedRequestParams, ProgressNotificationParam,
    ProtocolVersion, ServerCapabilities, ServerConfig, ServerNotification, ServerRequest, Tool,
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
use crate::sampling_convert::response::{
    CreateMessageResult, SamplingErrorData, SamplingResponse, StopReason,
};

use super::support;

pub(crate) const INSTRUCTIONS: &str = "the test server's instructions";

// One handler drives every transport test: tools pick the behaviour, so a case reads from the tool name alone
#[derive(Clone)]
pub(crate) struct TestServer {
    protocol_version: ProtocolVersion,
    supported: Vec<ProtocolVersion>,
    instructions: &'static str,
    schemas: bool,
    one_page: bool,
    pub(crate) listings: Arc<AtomicUsize>,
}

impl TestServer {
    pub(crate) fn standard() -> Self {
        Self {
            protocol_version: ProtocolVersion::LATEST_WITH_INITIALIZE,
            supported: ProtocolVersion::KNOWN_VERSIONS.to_vec(),
            instructions: INSTRUCTIONS,
            schemas: false,
            one_page: false,
            listings: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub(crate) fn legacy() -> Self {
        Self {
            protocol_version: ProtocolVersion::V_2025_06_18,
            supported: vec![ProtocolVersion::V_2025_06_18],
            ..Self::standard()
        }
    }

    pub(crate) fn with_output_schemas(mut self) -> Self {
        self.schemas = true;
        self
    }

    // A server whose first listing is complete and whose later ones have dropped the schema tools
    pub(crate) fn with_one_page(mut self) -> Self {
        self.one_page = true;
        self
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
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        // The SDK stamps a progress token on every request, listings included, so its absence is the framing bug
        if context.meta.get_progress_token().is_none() {
            return Err(ErrorData::invalid_params("missing progress token", None));
        }
        let previous = self.listings.fetch_add(1, Ordering::SeqCst);
        let cursor = request.and_then(|params| params.cursor);
        if self.one_page {
            let mut tools = vec![
                Tool::new("alpha", "the first tool", JsonObject::new()),
                Tool::new("beta", "the second tool", JsonObject::new()),
            ];
            if self.schemas && previous == 0 {
                tools.extend(schema_tools());
            }
            return Ok(ListToolsResult::with_all_items(tools));
        }
        let (mut tools, next) = match cursor.as_deref() {
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
        if self.schemas && cursor.is_none() {
            tools.extend(schema_tools());
        }
        let mut result = ListToolsResult::with_all_items(tools);
        result.next_cursor = next;
        Ok(result)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let outcome = match request.name.as_ref() {
            "echo" => CallToolResult::success(vec![ContentBlock::text("echoed")]),
            "fail" => CallToolResult::error(vec![ContentBlock::text("the tool failed")]),
            "rpc-error" => return Err(ErrorData::invalid_params("bad arguments", None)),
            "plain" => CallToolResult::success(vec![ContentBlock::text("plain")]),
            "schema-ok" => {
                let mut result = CallToolResult::success(vec![ContentBlock::text("structured")]);
                result.structured_content = Some(json!({"answer": 42}));
                result
            }
            "schema-missing" => CallToolResult::success(vec![ContentBlock::text("unstructured")]),
            "schema-invalid" => {
                let mut result = CallToolResult::success(vec![ContentBlock::text("wrong")]);
                result.structured_content = Some(json!({"answer": "no"}));
                result
            }
            "schema-error" => CallToolResult::error(vec![ContentBlock::text("the tool failed")]),
            "schema-broken" => {
                let mut result = CallToolResult::success(vec![ContentBlock::text("broken")]);
                result.structured_content = Some(json!({"answer": 42}));
                result
            }
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
            "sample-invalid" => {
                let request: ServerRequest = serde_json::from_value(json!({
                    "method": "sampling/createMessage",
                    "params": {
                        "messages": [{"role": "user", "content": {"type": "text", "text": "hello"}}],
                        "maxTokens": 16,
                        "metadata": 42
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
            "ping" => {
                let request: ServerRequest = serde_json::from_value(json!({"method": "ping"}))
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
            "protocol-version" => {
                let version = context
                    .extensions
                    .get::<http::request::Parts>()
                    .and_then(|parts| parts.headers.get("mcp-protocol-version"))
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("missing");
                CallToolResult::success(vec![ContentBlock::text(version)])
            }
            "capabilities" => {
                let text = context.peer.peer_info().map_or_else(
                    || "no peer info".to_owned(),
                    |info| serde_json::to_string(&info.capabilities).unwrap_or_default(),
                );
                CallToolResult::success(vec![ContentBlock::text(text)])
            }
            "client-info" => {
                let text = context.peer.peer_info().map_or_else(
                    || "no peer info".to_owned(),
                    |info| serde_json::to_string(&info.client_info).unwrap_or_default(),
                );
                CallToolResult::success(vec![ContentBlock::text(text)])
            }
            "log" => {
                let notification: ServerNotification = serde_json::from_value(json!({
                    "method": "notifications/message",
                    "params": {"level": "info", "data": "a log line"}
                }))
                .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
                context
                    .peer
                    .send_notification(notification)
                    .await
                    .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
                CallToolResult::success(vec![ContentBlock::text("logged")])
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

fn schema_tools() -> Vec<Tool> {
    let schema = Arc::new(output_schema());
    vec![
        Tool::new(
            "plain",
            "a tool without an output schema",
            JsonObject::new(),
        ),
        Tool::new(
            "schema-ok",
            "a tool with a matching output schema",
            JsonObject::new(),
        )
        .with_raw_output_schema(Arc::clone(&schema)),
        Tool::new(
            "schema-missing",
            "a tool that returns no structured content",
            JsonObject::new(),
        )
        .with_raw_output_schema(Arc::clone(&schema)),
        Tool::new(
            "schema-invalid",
            "a tool that returns the wrong structured content",
            JsonObject::new(),
        )
        .with_raw_output_schema(Arc::clone(&schema)),
        Tool::new(
            "schema-error",
            "a tool whose error result skips validation",
            JsonObject::new(),
        )
        .with_raw_output_schema(Arc::clone(&schema)),
        Tool::new(
            "schema-broken",
            "a tool whose declared output schema is invalid",
            JsonObject::new(),
        )
        .with_raw_output_schema(Arc::new(broken_output_schema())),
    ]
}

fn output_schema() -> JsonObject {
    serde_json::from_value(json!({
        "type": "object",
        "properties": {"answer": {"type": "integer"}},
        "required": ["answer"]
    }))
    .expect("the test output schema is an object")
}

fn broken_output_schema() -> JsonObject {
    serde_json::from_value(json!({"type": "not-a-json-schema-type"}))
        .expect("the broken test schema is an object")
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
    fn respond<'responder>(
        &'responder self,
        request: SamplingRequest,
    ) -> Pin<Box<dyn Future<Output = Result<SamplingResponse, ErrorData>> + Send + 'responder>>
    {
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

pub(crate) struct FailingSampling;

impl SamplingResponder for FailingSampling {
    fn respond<'responder>(
        &'responder self,
        _request: SamplingRequest,
    ) -> Pin<Box<dyn Future<Output = Result<SamplingResponse, ErrorData>> + Send + 'responder>>
    {
        Box::pin(async move {
            Ok(SamplingResponse::Error(SamplingErrorData {
                code: -32_099,
                message: "no model available".to_owned(),
            }))
        })
    }
}

pub(crate) struct TestElicitation;

impl ElicitationResponder for TestElicitation {
    fn respond<'responder>(
        &'responder self,
        request: ElicitationRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ElicitationResult, ErrorData>> + Send + 'responder>>
    {
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
