use std::future::{Future, ready};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use betterlite_bedrock::realtime_events::stream::{
    BedrockBidirectionalStream, ClosableBedrockRuntimeClient, StreamError,
};
use betterlite_bedrock::realtime_transport::runtime::{
    RealtimeClient, RealtimeRuntime, RuntimeConfig,
};
use betterlite_models::decisions::Json;
use secrecy::ExposeSecret as _;

use super::fakes::{FakeError, stream_error};
use super::streams::{ScriptedStream, StreamProfile};
use super::{Block, Run};

#[derive(Clone)]
pub(super) struct Plan {
    chunks: Vec<String>,
    block: Block,
    receive_failure: Option<usize>,
    send_failure: Option<usize>,
    open_error: bool,
}

impl Plan {
    pub(super) fn from_run(run: &Run) -> Self {
        Self {
            chunks: run.chunks.clone(),
            block: run.block,
            receive_failure: run.receive_failure,
            send_failure: run.send_failure,
            open_error: run.open_error,
        }
    }
}

fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Default)]
pub(super) struct Recorded {
    endpoint: Mutex<Option<String>>,
    region: Mutex<Option<String>>,
    access_key_id: Mutex<Option<String>>,
    secret_access_key: Mutex<Option<String>>,
    session_token: Mutex<Option<String>>,
    model_ids: Mutex<Vec<String>>,
    stream: Mutex<Option<StreamProfile>>,
    clients: Mutex<Vec<Arc<ClientState>>>,
    configured: AtomicBool,
}

impl Recorded {
    pub(super) fn endpoint(&self) -> Option<String> {
        lock(&self.endpoint).clone()
    }

    pub(super) fn region(&self) -> Option<String> {
        lock(&self.region).clone()
    }

    pub(super) fn identity(&self) -> (Option<String>, Option<String>, Option<String>) {
        (
            lock(&self.access_key_id).clone(),
            lock(&self.secret_access_key).clone(),
            lock(&self.session_token).clone(),
        )
    }

    pub(super) fn model_ids(&self) -> Vec<String> {
        lock(&self.model_ids).clone()
    }

    pub(super) fn sent_to_bedrock(&self) -> Vec<Json> {
        lock(&self.stream)
            .as_ref()
            .map_or_else(Vec::new, StreamProfile::sent)
    }

    pub(super) fn runtime_closed(&self) -> bool {
        lock(&self.clients)
            .last()
            .is_some_and(|client| client.closed.load(Ordering::SeqCst))
    }

    pub(super) fn input_closed(&self) -> bool {
        lock(&self.stream)
            .as_ref()
            .is_some_and(StreamProfile::closed)
    }

    pub(super) fn configured(&self) -> bool {
        self.configured.load(Ordering::SeqCst)
    }
}

pub(super) struct FakeRuntime {
    recorded: Arc<Recorded>,
    plan: Plan,
}

impl FakeRuntime {
    pub(super) const fn new(recorded: Arc<Recorded>, plan: Plan) -> Self {
        Self { recorded, plan }
    }
}

impl RealtimeRuntime for FakeRuntime {
    type Error = FakeError;

    fn connect(
        &self,
        config: RuntimeConfig,
    ) -> impl Future<
        Output = Result<Box<dyn RealtimeClient<Error = FakeError>>, StreamError<FakeError>>,
    > + Send {
        self.recorded.configured.store(true, Ordering::SeqCst);
        *lock(&self.recorded.endpoint) = Some(config.endpoint_uri);
        *lock(&self.recorded.region) = Some(config.region);
        let credentials = &config.credentials;
        *lock(&self.recorded.access_key_id) =
            Some(credentials.access_key_id.expose_secret().to_owned());
        *lock(&self.recorded.secret_access_key) =
            Some(credentials.secret_access_key.expose_secret().to_owned());
        *lock(&self.recorded.session_token) = credentials
            .session_token
            .as_ref()
            .map(|token| token.expose_secret().to_owned());
        let state = Arc::new(ClientState {
            closed: AtomicBool::new(false),
        });
        lock(&self.recorded.clients).push(Arc::clone(&state));
        let client: Box<dyn RealtimeClient<Error = FakeError>> = Box::new(FakeRuntimeClient {
            recorded: Arc::clone(&self.recorded),
            plan: self.plan.clone(),
            state,
        });
        ready(Ok(client))
    }
}

pub(super) struct ClientState {
    closed: AtomicBool,
}

struct FakeRuntimeClient {
    recorded: Arc<Recorded>,
    plan: Plan,
    state: Arc<ClientState>,
}

#[async_trait]
impl ClosableBedrockRuntimeClient for FakeRuntimeClient {
    type Error = FakeError;

    async fn close(&self) -> Result<(), StreamError<FakeError>> {
        self.state.closed.store(true, Ordering::SeqCst);
        Ok(())
    }
}

#[async_trait]
impl RealtimeClient for FakeRuntimeClient {
    type Error = FakeError;

    fn closable(&self) -> &dyn ClosableBedrockRuntimeClient<Error = FakeError> {
        self
    }

    async fn invoke(
        &self,
        model_id: &str,
    ) -> Result<Box<dyn BedrockBidirectionalStream<Error = FakeError>>, StreamError<FakeError>>
    {
        lock(&self.recorded.model_ids).push(model_id.to_owned());
        if self.plan.open_error {
            return Err(stream_error("the invoke failed", "StreamOpenError"));
        }
        let stream = ScriptedStream::new(
            self.plan.chunks.clone(),
            self.plan.block,
            self.plan.receive_failure,
            self.plan.send_failure,
        );
        *lock(&self.recorded.stream) = Some(stream.profile());
        Ok(Box::new(stream))
    }
}
