use std::future::Future;
use std::sync::{Arc, PoisonError};

use betterlite_mcp::client::sessions::error::ExitFailure;
use betterlite_mcp::client::sessions::negotiate::{
    ClientSession, InitializeOutcome, InitializeRequest,
};
use betterlite_mcp::client::sessions::runtime::SessionContext;
use betterlite_mcp::client::sessions::stream::MessageHandler;
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
            "transport.exit",
            self.plan.transport_exit,
            self.http,
            "transport exit",
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
    pub(crate) handler: MessageHandler,
}

impl FakeSession {
    fn push_request(&self, request: InitializeRequest) {
        self.script
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
    }
}

impl ClientSession for FakeSession {
    type Error = TestError;
    type InitializeResult = FakeInit;

    fn initialize(&mut self) -> impl Future<Output = Result<FakeInit, TestError>> + Send {
        std::future::ready(init_call(
            &self.script,
            &self.plan,
            "initialize",
            self.plan.initialize,
        ))
    }

    fn send_initialize(
        &mut self,
        request: InitializeRequest,
    ) -> impl Future<Output = Result<FakeInit, TestError>> + Send {
        self.push_request(request);
        std::future::ready(init_call(
            &self.script,
            &self.plan,
            "send_initialize",
            InitStep::Ok,
        ))
    }

    fn adopt(&mut self, _result: &FakeInit) {
        self.script.record("adopt");
    }

    fn send_initialized(&mut self) -> impl Future<Output = Result<(), TestError>> + Send {
        let script = Arc::clone(&self.script);
        async move {
            script.record("send_initialized");
            Ok(())
        }
    }
}

fn init_call(
    script: &Script,
    plan: &Plan,
    name: &'static str,
    fail: InitStep,
) -> Result<FakeInit, TestError> {
    script.record(name);
    match fail {
        InitStep::Fail => Err(TestError::Boom("initialize")),
        InitStep::Ok => Ok(fake_init(plan)),
    }
}

async fn exit_future(
    script: Arc<Script>,
    name: &'static str,
    step: Step,
    http: Option<FakeHttp>,
    label: &'static str,
) -> Result<(), ExitFailure<TestError>> {
    script.record(name);
    exit_result(step, http.as_ref(), label).await
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
    pub(crate) handler: MessageHandler,
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
        exit_future(
            self.script,
            "session.exit",
            self.plan.session_exit,
            None,
            "session exit",
        )
    }
}
