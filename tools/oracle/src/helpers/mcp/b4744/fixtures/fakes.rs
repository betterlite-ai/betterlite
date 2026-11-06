use std::future::Future;
use std::sync::{Arc, PoisonError};

use betterlite_mcp::client::cleanup::failure::ErrorContext;
use betterlite_mcp::client::sessions::error::{ExitFailure, McpFailure};
use betterlite_mcp::client::sessions::negotiate::{
    ClientSession, InitializeOutcome, InitializeRequest,
};
use betterlite_mcp::client::sessions::runtime::SessionContext;
use betterlite_mcp::client::sessions::stream::{MessageHandler, StreamFailure, StreamMessage};
use betterlite_mcp::client::sessions::transport::TransportContext;

use super::state::{FakeHttp, InitStep, Plan, Script, Step, TestError, enter_result, exit_result};

pub(crate) struct FakeTransport {
    pub(crate) script: Arc<Script>,
    pub(crate) plan: Arc<Plan>,
    pub(crate) http: Option<FakeHttp>,
}

impl TransportContext for FakeTransport {
    type Streams = ();
    type Error = TestError;

    fn enter(&mut self) -> impl Future<Output = Result<Self::Streams, Self::Error>> + Send {
        let script = Arc::clone(&self.script);
        let step = self.plan.transport_enter;
        async move {
            script.record("transport.enter");
            enter_result(step, "transport enter").await
        }
    }

    fn exit(self) -> impl Future<Output = Result<(), ExitFailure<TestError>>> + Send + 'static {
        exit_future(
            self.script,
            self.plan.transport_exit,
            self.http,
            Close::Transport,
        )
    }
}

pub(crate) struct FakeInit {
    pub(crate) version: String,
    pub(crate) instructions: Option<String>,
}

impl InitializeOutcome for FakeInit {
    fn protocol_version(&self) -> &str {
        &self.version
    }

    fn instructions(&self) -> Option<&str> {
        self.instructions.as_deref()
    }
}

pub(crate) struct FakeSession {
    pub(crate) script: Arc<Script>,
    pub(crate) plan: Arc<Plan>,
    pub(crate) handler: MessageHandler<TestError>,
}

impl ClientSession for FakeSession {
    type Error = TestError;
    type InitializeResult = FakeInit;

    fn initialize(&mut self) -> impl Future<Output = Result<FakeInit, TestError>> + Send {
        init_future(
            &self.script,
            &self.plan,
            &self.handler,
            "initialize",
            self.plan.initialize,
        )
    }

    fn send_initialize(
        &mut self,
        request: InitializeRequest,
    ) -> impl Future<Output = Result<FakeInit, TestError>> + Send {
        let script = Arc::clone(&self.script);
        let plan = Arc::clone(&self.plan);
        let handler = self.handler.clone();
        async move {
            script
                .requests
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(request);
            init_future(&script, &plan, &handler, "send_request", InitStep::Ok).await
        }
    }

    fn adopt(&mut self, _result: &FakeInit) {
        self.script.record("adopt");
    }

    fn send_initialized(&mut self) -> impl Future<Output = Result<(), TestError>> + Send {
        let script = Arc::clone(&self.script);
        async move {
            script.record("send_notification");
            Ok(())
        }
    }

    fn protocol_failure(error: &TestError) -> Option<McpFailure> {
        match error {
            TestError::Protocol(failure) => Some(failure.clone()),
            TestError::Boom(_) => None,
        }
    }
}

fn init_future(
    script: &Script,
    plan: &Plan,
    handler: &MessageHandler<TestError>,
    name: &'static str,
    step: InitStep,
) -> impl Future<Output = Result<FakeInit, TestError>> + Send + 'static {
    script.record(name);
    let protocol = |code: i32, context: Option<ErrorContext>| {
        TestError::Protocol(McpFailure {
            code,
            message: "upstream error".to_owned(),
            context,
        })
    };
    let result = match step {
        InitStep::Ok => Ok(fake_init(plan)),
        InitStep::Fail => Err(TestError::Boom("initialize")),
        InitStep::Protocol => Err(protocol(-32_000_i32, None)),
        InitStep::Timeout => Err(protocol(-32_001_i32, Some(ErrorContext::Timeout))),
        InitStep::StreamFailure => {
            let _failed = handler.handle(StreamMessage::Failure(StreamFailure::Http(Arc::new(
                TestError::Boom("stream"),
            ))));
            Err(protocol(-32_000_i32, None))
        }
    };
    std::future::ready(result)
}

#[derive(Clone, Copy)]
enum Close {
    Transport,
    Session,
}

async fn exit_future(
    script: Arc<Script>,
    step: Step,
    http: Option<FakeHttp>,
    close: Close,
) -> Result<(), ExitFailure<TestError>> {
    match close {
        Close::Transport => {
            script.record("transport.exit");
            exit_result(step, http.as_ref(), "transport exit").await
        }
        Close::Session => {
            script.record("session.exit");
            exit_result(step, http.as_ref(), "session exit").await
        }
    }
}

fn fake_init(plan: &Plan) -> FakeInit {
    FakeInit {
        version: plan.server_version.clone(),
        instructions: plan.instructions.clone(),
    }
}

pub(crate) struct FakeSessionContext {
    pub(crate) script: Arc<Script>,
    pub(crate) plan: Arc<Plan>,
    pub(crate) handler: MessageHandler<TestError>,
}

impl SessionContext for FakeSessionContext {
    type Session = FakeSession;
    type Error = TestError;

    fn enter(&mut self) -> impl Future<Output = Result<FakeSession, TestError>> + Send {
        let script = Arc::clone(&self.script);
        let plan = Arc::clone(&self.plan);
        let handler = self.handler.clone();
        async move {
            script.record("session.enter");
            match plan.session_enter {
                Step::Fail => Err(TestError::Boom("session enter")),
                Step::Ok | Step::Cancel | Step::Timeout | Step::RecordedFailure => {
                    Ok(FakeSession {
                        script,
                        plan,
                        handler,
                    })
                }
            }
        }
    }

    fn exit(self) -> impl Future<Output = Result<(), ExitFailure<TestError>>> + Send + 'static {
        exit_future(self.script, self.plan.session_exit, None, Close::Session)
    }
}
