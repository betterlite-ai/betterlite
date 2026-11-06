use std::future::Future;
use std::sync::{Arc, PoisonError};

use betterlite_mcp::client::sessions::error::ExitFailure;
use betterlite_mcp::client::sessions::factory::{HttpClientConfig, SseRequest};
use betterlite_mcp::client::sessions::runtime::{McpRuntime, SessionOptions};
use betterlite_mcp::client::sessions::stream::MessageHandler;
use betterlite_mcp::client::sessions::transport::StdioServerParameters;

mod fakes;
mod state;
pub(crate) use fakes::{FakeSession, FakeSessionContext, FakeTransport};
use state::{FakeHttp, exit_result};
pub(crate) use state::{InitStep, Plan, Script, Step, TestError};

pub(crate) struct Fixture {
    pub(crate) script: Arc<Script>,
    pub(crate) plan: Arc<Plan>,
    pub(crate) http: FakeHttp,
}

impl Fixture {
    pub(crate) fn new(plan: Plan) -> Self {
        Self {
            script: Arc::new(Script::default()),
            plan: Arc::new(plan),
            http: FakeHttp::default(),
        }
    }

    pub(crate) fn session_client(&self) -> FakeSession {
        FakeSession {
            script: Arc::clone(&self.script),
            plan: Arc::clone(&self.plan),
            handler: MessageHandler::default(),
        }
    }

    fn transport(&self, http: Option<FakeHttp>) -> FakeTransport {
        FakeTransport {
            script: Arc::clone(&self.script),
            plan: Arc::clone(&self.plan),
            http,
        }
    }

    pub(crate) fn contexts(
        &self,
        with_http: bool,
    ) -> betterlite_mcp::client::sessions::transport::TransportContexts<Self> {
        let http = with_http.then(|| self.http.clone());
        betterlite_mcp::client::sessions::transport::TransportContexts {
            transport: FakeTransport {
                script: Arc::clone(&self.script),
                plan: Arc::clone(&self.plan),
                http: http.clone(),
            },
            http_client: http.map(Arc::new),
        }
    }
}

fn record_slot<Value>(
    script: &Script,
    slot: &std::sync::Mutex<Option<Value>>,
    value: Value,
    name: &'static str,
) {
    script.record(name);
    *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(value);
}

impl McpRuntime for Fixture {
    type Error = TestError;
    type Streams = ();
    type Transport = FakeTransport;
    type HttpClient = FakeHttp;
    type Session = FakeSessionContext;

    fn stdio_client(&self, parameters: StdioServerParameters) -> FakeTransport {
        record_slot(&self.script, &self.script.stdio, parameters, "stdio_client");
        self.transport(None)
    }

    fn sse_client(&self, request: SseRequest) -> FakeTransport {
        record_slot(&self.script, &self.script.sse, request, "sse_client");
        self.transport(None)
    }

    fn streamable_http_client(&self, url: String, client: Arc<FakeHttp>) -> FakeTransport {
        self.script.record("streamable_http_client");
        self.script
            .urls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(url);
        FakeTransport {
            script: Arc::clone(&self.script),
            plan: Arc::clone(&self.plan),
            http: Some(FakeHttp::clone(&client)),
        }
    }

    fn http_client(&self, config: HttpClientConfig) -> Result<FakeHttp, TestError> {
        self.script.record("http_client");
        self.script
            .http
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(config);
        Ok(FakeHttp::clone(&self.http))
    }

    fn session_context(&self, _streams: (), options: SessionOptions) -> FakeSessionContext {
        FakeSessionContext {
            script: Arc::clone(&self.script),
            plan: Arc::clone(&self.plan),
            handler: options.message_handler,
        }
    }

    fn close_http_client(
        self: Arc<Self>,
        _client: Arc<FakeHttp>,
    ) -> impl Future<Output = Result<(), ExitFailure<TestError>>> + Send + 'static {
        self.script.record("http.close");
        let step = self.plan.http_close;
        async move { exit_result(step, None, "http close").await }
    }
}
