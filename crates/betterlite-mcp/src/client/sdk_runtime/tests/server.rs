use std::borrow::Cow;
use std::future::{Future, ready};
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use http::request::Parts;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ElicitationAction,
    ErrorData, GetPromptRequestParams, GetPromptResponse, GetPromptResult, Implementation,
    JsonObject, ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult,
    ListToolsResult, PaginatedRequestParams, ProgressNotificationParam, Prompt, PromptMessage,
    ProtocolVersion, ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource,
    ResourceContents, ResourceTemplate, Role, ServerCapabilities, ServerConfig, ServerNotification,
    ServerRequest, Tool,
};
use rmcp::service::{Peer, RequestContext};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{RoleServer, ServerHandler, ServiceError};
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

const SAMPLING_REQUEST: &str = r#"{"method":"sampling/createMessage","params":{"messages":[{"role":"user","content":{"type":"text","text":"hello"}}],"maxTokens":16}}"#;
const INVALID_SAMPLING_REQUEST: &str = r#"{"method":"sampling/createMessage","params":{"messages":[{"role":"user","content":{"type":"text","text":"hello"}}],"maxTokens":16,"metadata":42}}"#;
const ELICITATION_REQUEST: &str = r#"{"method":"elicitation/create","params":{"mode":"form","message":"who are you?","requestedSchema":{"type":"object","properties":{}}}}"#;
const PING_REQUEST: &str = r#"{"method":"ping"}"#;
const ROOTS_REQUEST: &str = r#"{"method":"roots/list"}"#;
const CUSTOM_REQUEST: &str = r#"{"method":"not/a-real-method","params":{}}"#;
const LOG_NOTIFICATION: &str =
    r#"{"method":"notifications/message","params":{"level":"info","data":"a log line"}}"#;

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

    fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> {
        // The SDK stamps a progress token on every request, listings included, so its absence is the framing bug
        if context.meta.get_progress_token().is_none() {
            return ready(Err(ErrorData::invalid_params(
                "missing progress token",
                None,
            )));
        }
        let listing = if self.one_page {
            one_page_listing(self)
        } else {
            paged_listing(self, request.as_ref())
        };
        ready(Ok(listing))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let fixed = fixed_result(&request.name);
        if let Some(result) = fixed {
            return Ok(CallToolResponse::from(result));
        }
        let outcome = tool_outcome(&request.name, &context).await?;
        Ok(CallToolResponse::from(outcome))
    }

    fn list_prompts(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListPromptsResult, ErrorData>> {
        ready(Ok(prompt_page(request)))
    }

    fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<GetPromptResponse, ErrorData>> {
        let name = request
            .arguments
            .as_ref()
            .and_then(|arguments| arguments.get("name"))
            .and_then(|value| value.as_str())
            .unwrap_or("stranger");
        let message = PromptMessage::new_text(Role::User, format!("hello {name}"));
        let result = GetPromptResult::new(vec![message]);
        ready(Ok(GetPromptResponse::from(result)))
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourcesResult, ErrorData>> {
        ready(Ok(resource_listing()))
    }

    fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourceTemplatesResult, ErrorData>> {
        ready(Ok(template_listing()))
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ReadResourceResponse, ErrorData>> {
        ready(Ok(read_response(request.uri)))
    }
}

fn prompt_page(request: Option<PaginatedRequestParams>) -> ListPromptsResult {
    let prompt = match request.and_then(|params| params.cursor).as_deref() {
        Some("more") => Prompt::new("farewell", Some("says goodbye"), None),
        _ => Prompt::new("greet", Some("says hello"), None),
    };
    ListPromptsResult::with_all_items(vec![prompt])
}

fn resource_listing() -> ListResourcesResult {
    ListResourcesResult::with_all_items(vec![Resource::new("file:///notes.txt", "notes")])
}

fn template_listing() -> ListResourceTemplatesResult {
    ListResourceTemplatesResult::with_all_items(vec![ResourceTemplate::new(
        "file:///{name}",
        "file",
    )])
}

fn read_response(uri: String) -> ReadResourceResponse {
    let contents = ResourceContents::text("the notes' text", uri);
    ReadResourceResponse::from(ReadResourceResult::new(vec![contents]))
}

fn one_page_listing(server: &TestServer) -> ListToolsResult {
    let previous = server.listings.fetch_add(1, Ordering::SeqCst);
    let mut tools = vec![
        Tool::new("alpha", "the first tool", JsonObject::new()),
        Tool::new("beta", "the second tool", JsonObject::new()),
    ];
    if server.schemas && previous == 0 {
        tools.extend(schema_tools());
    }
    ListToolsResult::with_all_items(tools)
}

fn paged_listing(server: &TestServer, request: Option<&PaginatedRequestParams>) -> ListToolsResult {
    server.listings.fetch_add(1, Ordering::SeqCst);
    let cursor = request.and_then(|params| params.cursor.as_deref());
    let (mut tools, next) = match cursor {
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
    if server.schemas && cursor.is_none() {
        tools.extend(schema_tools());
    }
    let mut result = ListToolsResult::with_all_items(tools);
    result.next_cursor = next;
    result
}

async fn tool_outcome(
    name: &str,
    context: &RequestContext<RoleServer>,
) -> Result<CallToolResult, ErrorData> {
    match name {
        "rpc-error" => Err(ErrorData::invalid_params("bad arguments", None)),
        "progress" => progress_result(context).await,
        "slow" => Ok(slow_result().await),
        "log" => logged_result(&context.peer).await,
        "protocol-version" => Ok(version_result(context)),
        "capabilities" => Ok(peer_capabilities(context)),
        "client-info" => Ok(peer_client_info(context)),
        "sample" => ask_peer(SAMPLING_REQUEST, context).await,
        "sample-invalid" => ask_peer(INVALID_SAMPLING_REQUEST, context).await,
        "elicit" => ask_peer(ELICITATION_REQUEST, context).await,
        "ping" => ask_peer(PING_REQUEST, context).await,
        "roots" => ask_peer(ROOTS_REQUEST, context).await,
        "custom" => ask_peer(CUSTOM_REQUEST, context).await,
        other => Err(ErrorData::invalid_params(
            format!("unknown tool {other}"),
            None,
        )),
    }
}

fn fixed_result(name: &str) -> Option<CallToolResult> {
    let result = match name {
        "echo" => CallToolResult::success(vec![ContentBlock::text("echoed")]),
        "fail" | "schema-error" => {
            CallToolResult::error(vec![ContentBlock::text("the tool failed")])
        }
        "plain" => CallToolResult::success(vec![ContentBlock::text("plain")]),
        "schema-missing" => CallToolResult::success(vec![ContentBlock::text("unstructured")]),
        "schema-ok" => structured_result("structured", r#"{"answer": 42}"#),
        "schema-invalid" => structured_result("wrong", r#"{"answer": "no"}"#),
        "schema-broken" => structured_result("broken", r#"{"answer": 42}"#),
        _ => return None,
    };
    Some(result)
}

fn structured_result(text: &str, content: &str) -> CallToolResult {
    let mut result = CallToolResult::success(vec![ContentBlock::text(text)]);
    result.structured_content = serde_json::from_str(content).ok();
    result
}

async fn progress_result(
    context: &RequestContext<RoleServer>,
) -> Result<CallToolResult, ErrorData> {
    let Some(token) = context.meta.get_progress_token() else {
        return Err(ErrorData::invalid_params("missing progress token", None));
    };
    for (fraction, total) in [(25.0_f64, 100.0_f64), (75.0_f64, 100.0_f64)] {
        let update = ProgressNotificationParam::new(token.clone(), fraction)
            .with_total(total)
            .with_message("working");
        if let Err(error) = context.peer.notify_progress(update).await {
            return Err(ErrorData::internal_error(error.to_string(), None));
        }
    }
    Ok(CallToolResult::success(vec![ContentBlock::text(
        "progressed",
    )]))
}

async fn slow_result() -> CallToolResult {
    sleep(support::CALL_TIMEOUT.saturating_mul(6)).await;
    CallToolResult::success(vec![ContentBlock::text("too late")])
}

async fn logged_result(peer: &Peer<RoleServer>) -> Result<CallToolResult, ErrorData> {
    let notification: ServerNotification = serde_json::from_str(LOG_NOTIFICATION)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    let sent = peer.send_notification(notification).await;
    if let Err(error) = sent {
        return Err(ErrorData::internal_error(error.to_string(), None));
    }
    Ok(CallToolResult::success(vec![ContentBlock::text("logged")]))
}

fn version_result(context: &RequestContext<RoleServer>) -> CallToolResult {
    let version = context
        .extensions
        .get::<Parts>()
        .and_then(|parts| parts.headers.get("mcp-protocol-version"))
        .and_then(|value| value.to_str().ok())
        .unwrap_or("missing");
    CallToolResult::success(vec![ContentBlock::text(version)])
}

fn peer_capabilities(context: &RequestContext<RoleServer>) -> CallToolResult {
    let text = context.peer.peer_info().map_or_else(
        || "no peer info".to_owned(),
        |info| serde_json::to_string(&info.capabilities).unwrap_or_default(),
    );
    CallToolResult::success(vec![ContentBlock::text(text)])
}

fn peer_client_info(context: &RequestContext<RoleServer>) -> CallToolResult {
    let text = context.peer.peer_info().map_or_else(
        || "no peer info".to_owned(),
        |info| serde_json::to_string(&info.client_info).unwrap_or_default(),
    );
    CallToolResult::success(vec![ContentBlock::text(text)])
}

async fn ask_peer(
    request: &str,
    context: &RequestContext<RoleServer>,
) -> Result<CallToolResult, ErrorData> {
    let incoming: ServerRequest = serde_json::from_str(request)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    let answer = peer_answer(context, incoming).await;
    Ok(CallToolResult::success(vec![ContentBlock::text(answer)]))
}

fn schema_tools() -> Vec<Tool> {
    let schema = Arc::new(output_schema());
    vec![
        Tool::new(
            "plain",
            "a tool without an output schema",
            JsonObject::new(),
        ),
        schema_tool("schema-ok", "a tool with a matching output schema", &schema),
        schema_tool(
            "schema-missing",
            "a tool that returns no structured content",
            &schema,
        ),
        schema_tool(
            "schema-invalid",
            "a tool that returns the wrong structured content",
            &schema,
        ),
        schema_tool(
            "schema-error",
            "a tool whose error result skips validation",
            &schema,
        ),
        Tool::new(
            "schema-broken",
            "a tool whose declared output schema is invalid",
            JsonObject::new(),
        )
        .with_raw_output_schema(Arc::new(broken_output_schema())),
    ]
}

fn schema_tool(name: &'static str, description: &'static str, schema: &Arc<JsonObject>) -> Tool {
    Tool::new(name, description, JsonObject::new()).with_raw_output_schema(Arc::clone(schema))
}

fn output_schema() -> JsonObject {
    serde_json::from_str(
        r#"{"type":"object","properties":{"answer":{"type":"integer"}},"required":["answer"]}"#,
    )
    .expect("the test output schema is an object")
}

fn broken_output_schema() -> JsonObject {
    serde_json::from_str(r#"{"type":"not-a-json-schema-type"}"#)
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
        StreamableHttpService::new(move || Ok(server.clone()), Arc::default(), config);
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
    content.insert(
        "answer".to_owned(),
        serde_json::to_value(message).unwrap_or_default(),
    );
    ElicitationResult {
        action: ElicitationAction::Accept,
        content: Some(content),
    }
}
