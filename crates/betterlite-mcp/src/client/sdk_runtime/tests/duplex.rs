use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures_util::FutureExt as _;
use rmcp::model::{
    GetPromptResult, ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult,
    ReadResourceResult, ResourceContents,
};
use rmcp::service::{RxJsonRpcMessage, TxJsonRpcMessage};
use rmcp::transport::Transport;
use rmcp::transport::async_rw::AsyncRwTransport;
use rmcp::{RoleClient, ServiceExt as _};
use tokio::io::{DuplexStream, ReadHalf, WriteHalf, duplex, split};
use tokio::task::JoinSet;
use tokio::time::timeout;

use crate::client::cleanup::http_client::CleanupScope;
use crate::client::sdk_runtime::session::calls::WireRequest;
use crate::client::sdk_runtime::session::{SdkSession, SdkSessionContext};
use crate::client::sdk_transport::error::SdkError;
use crate::client::sessions::client::SessionClient;
use crate::client::sessions::error::{ExitFailure, SessionError};
use crate::client::sessions::factory::{HttpClientConfig, SseRequest};
use crate::client::sessions::run::run_with_session;
use crate::client::sessions::runtime::{HttpClientHandle, McpRuntime, SessionOptions};
use crate::client::sessions::stream::MessageHandler;
use crate::client::sessions::transport::{StdioServerParameters, TransportContext, TransportType};
use crate::client::tools::calling::{CallToolOptions, call_tool};
use crate::client::tools::listing::list_tools;
use crate::client::tools::result::{ContentBlock, ToolCallResult};
use crate::tools::tool_calls::CallRequest;

use super::server::TestServer;
use super::support::{self, RecordingHost};

type DuplexRw = AsyncRwTransport<RoleClient, ReadHalf<DuplexStream>, WriteHalf<DuplexStream>>;

pub(crate) struct DuplexStreams(DuplexRw);

impl Transport<RoleClient> for DuplexStreams {
    type Error = SdkError;

    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = Result<(), SdkError>> + Send + 'static {
        self.0
            .send(item)
            .map(|sent| sent.map_err(SdkError::from))
            .boxed()
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleClient>> {
        self.0.receive().await
    }

    async fn close(&mut self) -> Result<(), SdkError> {
        self.0.close().await.map_err(SdkError::from)
    }
}

pub(crate) struct DuplexContext {
    streams: Option<DuplexStreams>,
}

impl TransportContext for DuplexContext {
    type Streams = DuplexStreams;
    type Error = SdkError;

    fn enter(&mut self) -> impl Future<Output = Result<DuplexStreams, SdkError>> {
        ready(self.streams.take().ok_or(SdkError::AlreadyOpen))
    }

    fn exit(self) -> impl Future<Output = Result<(), ExitFailure<SdkError>>> + Send + 'static {
        ready(Ok(()))
    }
}

pub(crate) struct NoHttpClient;

impl HttpClientHandle for NoHttpClient {
    type Error = SdkError;

    fn begin_cleanup(&self, _scope: CleanupScope) {}

    fn finish_cleanup(&self) -> Vec<Arc<SdkError>> {
        Vec::new()
    }
}

pub(crate) struct DuplexRuntime {
    servers: Mutex<JoinSet<()>>,
}

impl McpRuntime for DuplexRuntime {
    type Error = SdkError;
    type Streams = DuplexStreams;
    type Transport = DuplexContext;
    type HttpClient = NoHttpClient;
    type Session = SdkSessionContext<DuplexStreams>;

    // Every operation opens its own session, so each one gets a fresh server on the other end of a new duplex
    fn stdio_client(&self, _parameters: StdioServerParameters) -> DuplexContext {
        let (client_half, server_half) = duplex(64 * 1024);
        self.servers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .spawn(async move {
                let running = TestServer::standard().serve(server_half).await.unwrap();
                drop(running.waiting().await);
            });
        let (read, write) = split(client_half);
        DuplexContext {
            streams: Some(DuplexStreams(AsyncRwTransport::new(read, write))),
        }
    }

    fn sse_client(&self, _request: SseRequest) -> DuplexContext {
        // the duplex runtime serves stdio sessions only, so the other transports arrive without streams
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
        SdkSessionContext::new(streams, options, MessageHandler::default(), None)
    }

    fn close_http_client(
        self: Arc<Self>,
        _client: Arc<NoHttpClient>,
    ) -> impl Future<Output = Result<(), ExitFailure<SdkError>>> + Send + 'static {
        ready(Ok(()))
    }
}

fn duplex_client(read_timeout: Duration) -> SessionClient<DuplexRuntime> {
    let runtime = DuplexRuntime {
        servers: Mutex::new(JoinSet::new()),
    };
    support::client_on(
        Arc::new(runtime),
        support::settings(TransportType::Stdio),
        "",
        read_timeout,
    )
}

#[tokio::test]
async fn the_session_lists_and_calls_tools_over_an_in_process_duplex() {
    let mut client = duplex_client(support::CALL_TIMEOUT);
    let tools = list_tools(&mut client, support::CALL_TIMEOUT, true)
        .await
        .unwrap();
    let names: Vec<String> = tools.into_iter().map(|tool| tool.name).collect();
    assert_eq!(names, ["alpha", "beta", "gamma"]);
    let request = CallRequest {
        name: "echo".to_owned(),
        arguments: None,
    };
    let result = Box::pin(call_tool(
        &mut client,
        &request,
        None::<&RecordingHost>,
        CallToolOptions::default(),
    ))
    .await
    .unwrap();
    let ToolCallResult::Complete(complete) = result else {
        panic!("expected a complete tool result");
    };
    match complete.content.as_slice() {
        [ContentBlock::Text(text)] => assert_eq!(text.text, "echoed"),
        other => panic!("unexpected content blocks: {other:?}"),
    }
    assert_eq!(
        client.last_initialize_instructions.as_deref(),
        Some(super::server::INSTRUCTIONS)
    );
}

#[tokio::test(start_paused = true)]
async fn a_read_timeout_that_fires_before_the_answer_is_a_read_timeout() {
    let mut client = duplex_client(Duration::from_millis(200));
    let request = CallRequest {
        name: "slow".to_owned(),
        arguments: None,
    };
    let run = call_tool(
        &mut client,
        &request,
        None::<&RecordingHost>,
        CallToolOptions {
            raise_on_error: true,
            allow_input_required: false,
        },
    );
    let error = Box::pin(timeout(Duration::from_secs(30), run))
        .await
        .expect("the call finished")
        .unwrap_err();
    assert!(matches!(error, SessionError::ReadTimeout(_)), "{error}");
}

#[tokio::test]
async fn the_wire_helpers_carry_the_prompt_and_resource_results() {
    let mut client = duplex_client(support::CALL_TIMEOUT);
    run_with_session(
        &mut client,
        async |session| {
            check_prompt_wire(session).await;
            check_resource_wire(session).await;
            Ok(())
        },
        false,
    )
    .await
    .unwrap();
}

async fn check_prompt_wire(session: &SdkSession) {
    for (cursor, expected) in [(None, "greet"), (Some("more"), "farewell")] {
        let wire = session
            .wire_request(WireRequest::ListPrompts(cursor))
            .await
            .expect("the prompt page");
        let page: ListPromptsResult = serde_json::from_str(&wire).expect("the prompt page decodes");
        assert_eq!(page.prompts[0].name, expected);
    }
    let mut arguments = BTreeMap::new();
    arguments.insert("name".to_owned(), "ada".to_owned());
    let greeting: GetPromptResult = serde_json::from_str(
        &session
            .get_prompt_wire("greet", Some(&arguments))
            .await
            .expect("the greeting prompt"),
    )
    .expect("the greeting prompt decodes");
    let text = greeting.messages[0]
        .content
        .as_text()
        .expect("a text prompt message");
    assert_eq!(text.text, "hello ada");
}

async fn check_resource_wire(session: &SdkSession) {
    let resources: ListResourcesResult = serde_json::from_str(
        &session
            .wire_request(WireRequest::ListResources(None))
            .await
            .expect("the resource page"),
    )
    .expect("the resource page decodes");
    assert_eq!(resources.resources[0].uri, "file:///notes.txt");
    let templates: ListResourceTemplatesResult = serde_json::from_str(
        &session
            .wire_request(WireRequest::ListResourceTemplates(None))
            .await
            .expect("the template page"),
    )
    .expect("the template page decodes");
    assert_eq!(templates.resource_templates[0].name, "file");
    let contents: ReadResourceResult = serde_json::from_str(
        &session
            .wire_request(WireRequest::ReadResource("file:///notes.txt"))
            .await
            .expect("the resource contents"),
    )
    .expect("the resource contents decode");
    let ResourceContents::TextResourceContents { uri, text, .. } = &contents.contents[0] else {
        panic!("the notes resource answers text contents");
    };
    assert_eq!(uri, "file:///notes.txt");
    assert_eq!(text, "the notes' text");
}
