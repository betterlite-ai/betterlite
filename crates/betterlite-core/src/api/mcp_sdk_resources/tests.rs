#![cfg(test)]

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_mcp::client::cleanup::http_client::CleanupScope;
use betterlite_mcp::client::sdk_runtime::handler::ServerRequestHandlers;
use betterlite_mcp::client::sdk_runtime::session::{SdkSession, SdkSessionContext};
use betterlite_mcp::client::sdk_transport::error::SdkError;
use betterlite_mcp::client::sessions::client::{SessionClient, SessionSettings};
use betterlite_mcp::client::sessions::error::ExitFailure;
use betterlite_mcp::client::sessions::factory::{HttpClientConfig, SseRequest};
use betterlite_mcp::client::sessions::negotiate::{Callbacks, ProtocolVersion};
use betterlite_mcp::client::sessions::run::run_with_session;
use betterlite_mcp::client::sessions::runtime::{HttpClientHandle, McpRuntime, SessionOptions};
use betterlite_mcp::client::sessions::stream::MessageHandler;
use betterlite_mcp::client::sessions::transport::{
    StdioConfig, StdioServerParameters, TransportContext, TransportType,
};
use betterlite_mcp::credentials::{McpAuthType, McpClient as CredentialClient};
use futures_util::FutureExt as _;
use rmcp::model::{
    ErrorData, GetPromptRequestParams as SdkGetPromptParams, GetPromptResponse, GetPromptResult,
    Implementation, ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult,
    PaginatedRequestParams, Prompt, PromptMessage, ProtocolVersion as SdkVersion,
    ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource,
    ResourceContents, ResourceTemplate, Role, ServerCapabilities as SdkCapabilities, ServerConfig,
};
use rmcp::service::{MaybeSendFuture, RequestContext, RxJsonRpcMessage, TxJsonRpcMessage};
use rmcp::transport::Transport;
use rmcp::transport::async_rw::AsyncRwTransport;
use rmcp::{RoleClient, RoleServer, ServerHandler, ServiceExt as _};
use tokio::io::{DuplexStream, ReadHalf, WriteHalf, duplex, split};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use url::Url;

use crate::api::mcp_client_resources::session::ResourceSession;
use crate::api::mcp_client_resources::types::{Capability, GetPromptRequestParams, PromptRole};

type DuplexRw = AsyncRwTransport<RoleClient, ReadHalf<DuplexStream>, WriteHalf<DuplexStream>>;

struct DuplexStreams(DuplexRw);

impl Transport<RoleClient> for DuplexStreams {
    type Error = SdkError;

    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = Result<(), SdkError>> + Send + 'static {
        self.0.send(item).map(|sent| sent.map_err(SdkError::from))
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleClient>> {
        self.0.receive().await
    }

    async fn close(&mut self) -> Result<(), SdkError> {
        self.0.close().await.map_err(SdkError::from)
    }
}

struct DuplexContext {
    streams: Option<DuplexStreams>,
}

impl TransportContext for DuplexContext {
    type Streams = DuplexStreams;
    type Error = SdkError;

    fn enter(&mut self) -> impl Future<Output = Result<DuplexStreams, SdkError>> + Send {
        ready(self.streams.take().ok_or(SdkError::AlreadyOpen))
    }

    fn exit(self) -> impl Future<Output = Result<(), ExitFailure<SdkError>>> + Send + 'static {
        ready(Ok(()))
    }
}

struct NoHttpClient;

impl HttpClientHandle for NoHttpClient {
    type Error = SdkError;

    fn begin_cleanup(&self, _scope: CleanupScope) {}

    fn finish_cleanup(&self) -> Vec<Arc<SdkError>> {
        Vec::new()
    }
}

struct DuplexRuntime {
    streams: Mutex<Option<DuplexStreams>>,
    handler: MessageHandler<SdkError>,
}

impl McpRuntime for DuplexRuntime {
    type Error = SdkError;
    type Streams = DuplexStreams;
    type Transport = DuplexContext;
    type HttpClient = NoHttpClient;
    type Session = SdkSessionContext<DuplexStreams>;

    fn stdio_client(&self, _parameters: StdioServerParameters) -> DuplexContext {
        let streams = self
            .streams
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        DuplexContext { streams }
    }

    // the duplex runtime serves stdio sessions only, and this test opens no other transport
    fn sse_client(&self, _request: SseRequest) -> DuplexContext {
        DuplexContext { streams: None }
    }

    fn streamable_http_client(&self, _url: String, _client: Arc<NoHttpClient>) -> DuplexContext {
        DuplexContext { streams: None }
    }

    fn http_client(&self, _config: HttpClientConfig) -> Result<NoHttpClient, SdkError> {
        Ok(NoHttpClient)
    }

    fn session_context(
        &self,
        streams: DuplexStreams,
        options: SessionOptions<SdkError>,
    ) -> SdkSessionContext<DuplexStreams> {
        SdkSessionContext::new(streams, options, self.handler.clone(), None)
    }

    fn close_http_client(
        self: Arc<Self>,
        _client: Arc<NoHttpClient>,
    ) -> impl Future<Output = Result<(), ExitFailure<SdkError>>> + Send + 'static {
        ready(Ok(()))
    }
}

#[derive(Clone)]
struct ResourceServer;

impl ServerHandler for ResourceServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            SdkCapabilities::builder()
                .enable_prompts()
                .enable_prompts_list_changed()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new("betterlite-resource-test", "1.0"))
        .with_protocol_version(SdkVersion::LATEST_WITH_INITIALIZE)
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [SdkVersion]> {
        Cow::Owned(SdkVersion::KNOWN_VERSIONS.to_vec())
    }

    fn list_prompts(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListPromptsResult, ErrorData>> + MaybeSendFuture + '_ {
        let cursor = request.and_then(|params| params.cursor);
        let prompt = match cursor.as_deref() {
            Some("more") => Prompt::new("farewell", Some("says goodbye"), None),
            _ => Prompt::new("greet", Some("says hello"), None),
        };
        let mut result = ListPromptsResult::with_all_items(vec![prompt]);
        result.next_cursor = cursor.is_none().then(|| "more".to_owned());
        ready(Ok(result))
    }

    fn get_prompt(
        &self,
        request: SdkGetPromptParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<GetPromptResponse, ErrorData>> + MaybeSendFuture + '_ {
        if request.name != "greet" {
            return ready(Err(ErrorData::invalid_params("no such prompt", None)));
        }
        let name = request
            .arguments
            .as_ref()
            .and_then(|arguments| arguments.get("name"))
            .and_then(|value| value.as_str())
            .unwrap_or("stranger");
        let message = PromptMessage::new_text(Role::User, format!("hello {name}"));
        let result = GetPromptResult::new(vec![message]).with_description("a greeting");
        ready(Ok(GetPromptResponse::from(result)))
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourcesResult, ErrorData>> + MaybeSendFuture + '_ {
        ready(Ok(resource_page()))
    }

    fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourceTemplatesResult, ErrorData>> + MaybeSendFuture + '_
    {
        ready(Ok(template_page()))
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ReadResourceResponse, ErrorData>> + MaybeSendFuture + '_ {
        ready(Ok(notes_response(request.uri)))
    }
}

fn notes_resource() -> Resource {
    Resource::new("file:///notes.txt", "notes").with_mime_type("text/plain")
}

fn file_template() -> ResourceTemplate {
    ResourceTemplate::new("file:///{name}", "file")
}

fn resource_page() -> ListResourcesResult {
    ListResourcesResult::with_all_items(vec![notes_resource()])
}

fn template_page() -> ListResourceTemplatesResult {
    ListResourceTemplatesResult::with_all_items(vec![file_template()])
}

fn notes_response(uri: String) -> ReadResourceResponse {
    let contents = ResourceContents::text("the notes' text", uri);
    ReadResourceResponse::from(ReadResourceResult::new(vec![contents]))
}

fn session_client(runtime: DuplexRuntime) -> SessionClient<DuplexRuntime> {
    SessionClient {
        runtime: Arc::new(runtime),
        auth: CredentialClient::new(
            String::new(),
            McpAuthType::None,
            None,
            None,
            BTreeMap::new(),
            Duration::from_secs(10),
            None,
            None,
        ),
        settings: SessionSettings {
            transport_type: TransportType::Stdio,
            stdio: Some(StdioConfig {
                command: "/bin/true".to_owned(),
                args: Vec::new(),
                env: None,
            }),
            environment: BTreeMap::new(),
            npm_cache_dir: String::new(),
            tls: TlsPolicy {
                verification: Verification::Enabled,
                ca_file: None,
                certificate: None,
                http2: false,
                ecdh_curve: None,
                user_agent: "betterlite".to_owned(),
            },
            protocol_version: ProtocolVersion::Auto,
            client_version: "1.105.1".to_owned(),
            callbacks: Callbacks::default(),
            handlers: ServerRequestHandlers::default(),
        },
        tracker: TaskTracker::new(),
        caller_cancelled: CancellationToken::new(),
        last_initialize_instructions: None,
    }
}

#[tokio::test]
async fn every_resource_session_method_round_trips_through_the_server() {
    let (client_half, server_half) = duplex(64 * 1024);
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        let running = ResourceServer.serve(server_half).await.unwrap();
        drop(running.waiting().await);
    });
    let (read, write) = split(client_half);
    let runtime = DuplexRuntime {
        streams: Mutex::new(Some(DuplexStreams(AsyncRwTransport::new(read, write)))),
        handler: MessageHandler::default(),
    };
    let mut client = session_client(runtime);
    run_with_session(
        &mut client,
        async |session| {
            check_capabilities(session);
            check_prompt_pages(session).await;
            check_prompt_requests(session).await;
            check_resources(session).await;
            Ok(())
        },
        false,
    )
    .await
    .unwrap();
    tasks.shutdown().await;
}

fn check_capabilities(session: &SdkSession) {
    let capabilities = session
        .server_capabilities()
        .expect("the handshake adopted the capabilities");
    assert_eq!(
        capabilities.prompts,
        Some(Capability {
            list_changed: Some(true),
        })
    );
    assert_eq!(
        capabilities.resources,
        Some(Capability { list_changed: None })
    );
}

async fn check_prompt_pages(session: &mut SdkSession) {
    let first = session
        .list_prompts(None)
        .await
        .expect("the first prompt page");
    assert_eq!(first.prompts.len(), 1);
    assert_eq!(first.prompts[0].name, "greet");
    assert_eq!(first.prompts[0].description.as_deref(), Some("says hello"));
    assert_eq!(first.next_cursor.as_deref(), Some("more"));
    let second = session
        .list_prompts(first.next_cursor.clone())
        .await
        .expect("the second prompt page");
    assert_eq!(second.prompts[0].name, "farewell");
    assert_eq!(second.next_cursor, None);
}

async fn check_prompt_requests(session: &mut SdkSession) {
    let mut arguments = BTreeMap::new();
    arguments.insert("name".to_owned(), "ada".to_owned());
    let prompt = session
        .get_prompt(GetPromptRequestParams {
            name: "greet".to_owned(),
            arguments: Some(arguments),
        })
        .await
        .expect("the greeting prompt");
    assert_eq!(prompt.description.as_deref(), Some("a greeting"));
    assert_eq!(prompt.messages[0].role, PromptRole::User);
    assert_eq!(
        prompt.messages[0]
            .content
            .get("text")
            .and_then(|text| text.as_str()),
        Some("hello ada")
    );
    let rejected = session
        .get_prompt(GetPromptRequestParams {
            name: "missing".to_owned(),
            arguments: None,
        })
        .await
        .expect_err("the server rejects an unknown prompt");
    let SdkError::JsonRpc(failure) = rejected else {
        panic!("expected a JSON-RPC failure");
    };
    assert_eq!(failure.code.0, -32_602_i32);
    assert_eq!(failure.message.as_ref(), "no such prompt");
}

async fn check_resources(session: &mut SdkSession) {
    let resources = session
        .list_resources(None)
        .await
        .expect("the resource page");
    assert_eq!(resources.resources.len(), 1);
    assert_eq!(resources.resources[0].uri, "file:///notes.txt");
    assert_eq!(resources.resources[0].name, "notes");
    assert_eq!(
        resources.resources[0].mime_type.as_deref(),
        Some("text/plain")
    );
    let templates = session
        .list_resource_templates(None)
        .await
        .expect("the template page");
    assert_eq!(templates.resource_templates.len(), 1);
    assert_eq!(
        templates.resource_templates[0].uri_template,
        "file:///{name}"
    );
    assert_eq!(templates.resource_templates[0].name, "file");
    let contents = session
        .read_resource(Url::parse("file:///notes.txt").unwrap())
        .await
        .expect("the notes resource");
    assert_eq!(contents.contents[0].uri, "file:///notes.txt");
    assert_eq!(
        contents.contents[0].text.as_deref(),
        Some("the notes' text")
    );
    assert_eq!(
        contents.contents[0].mime_type.as_deref(),
        Some("text/plain")
    );
}

#[test]
fn a_json_rpc_failure_keeps_its_code_and_other_errors_do_not() {
    let rejected = SdkError::JsonRpc(ErrorData::invalid_params("no such prompt", None));
    let failure = <SdkSession as ResourceSession>::protocol_failure(&rejected).expect("mapped");
    assert_eq!(failure.code, -32_602_i32);
    assert_eq!(failure.message, "no such prompt");
    assert!(
        <SdkSession as ResourceSession>::protocol_failure(&SdkError::AlreadyOpen).is_none(),
        "a transport error carries no JSON-RPC code"
    );
}
