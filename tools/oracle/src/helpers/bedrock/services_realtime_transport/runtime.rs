use std::future::{Future, ready};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

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

pub(super) struct RecordedView {
    pub(super) endpoint: Option<String>,
    pub(super) region: Option<String>,
    pub(super) model_ids: Vec<String>,
    pub(super) configured: bool,
}

impl Recorded {
    pub(super) fn view(&self) -> RecordedView {
        RecordedView {
            endpoint: self
                .endpoint
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
            region: self
                .region
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
            model_ids: self
                .model_ids
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
            configured: self.configured.load(Ordering::SeqCst),
        }
    }

    pub(super) fn identity(&self) -> (Option<String>, Option<String>, Option<String>) {
        (
            self.access_key_id
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
            self.secret_access_key
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
            self.session_token
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
    }

    pub(super) fn sent_to_bedrock(&self) -> Vec<Json> {
        self.stream
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map_or_else(Vec::new, StreamProfile::sent)
    }

    pub(super) fn runtime_closed(&self) -> bool {
        self.clients
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .last()
            .is_some_and(|client| client.closed.load(Ordering::SeqCst))
    }

    pub(super) fn input_closed(&self) -> bool {
        self.stream
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_some_and(|profile| profile.closed.load(Ordering::SeqCst))
    }
}

pub(super) struct FakeRuntime {
    pub(super) recorded: Arc<Recorded>,
    pub(super) plan: Plan,
}

impl RealtimeRuntime for FakeRuntime {
    type Error = FakeError;

    fn connect(
        &self,
        config: RuntimeConfig,
    ) -> impl Future<
        Output = Result<Box<dyn RealtimeClient<Error = FakeError>>, StreamError<FakeError>>,
    > + Send {
        let state = record_connection(&self.recorded, config);
        let client: Box<dyn RealtimeClient<Error = FakeError>> = Box::new(FakeRuntimeClient {
            recorded: Arc::clone(&self.recorded),
            plan: self.plan.clone(),
            state,
        });
        ready(Ok(client))
    }
}

fn record_connection(recorded: &Recorded, config: RuntimeConfig) -> Arc<ClientState> {
    recorded.configured.store(true, Ordering::SeqCst);
    *recorded
        .endpoint
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(config.endpoint_uri);
    *recorded
        .region
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(config.region);
    let credentials = &config.credentials;
    *recorded
        .access_key_id
        .lock()
        .unwrap_or_else(PoisonError::into_inner) =
        Some(credentials.access_key_id.expose_secret().to_owned());
    *recorded
        .secret_access_key
        .lock()
        .unwrap_or_else(PoisonError::into_inner) =
        Some(credentials.secret_access_key.expose_secret().to_owned());
    *recorded
        .session_token
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = credentials
        .session_token
        .as_ref()
        .map(|token| token.expose_secret().to_owned());
    let state = Arc::new(ClientState {
        closed: AtomicBool::new(false),
    });
    recorded
        .clients
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(Arc::clone(&state));
    state
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
        self.recorded
            .model_ids
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(model_id.to_owned());
        if self.plan.open_error {
            return Err(stream_error("the invoke failed", "StreamOpenError"));
        }
        let stream = ScriptedStream::new(
            self.plan.chunks.clone(),
            self.plan.block,
            self.plan.receive_failure,
            self.plan.send_failure,
        );
        *self
            .recorded
            .stream
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(stream.profile());
        Ok(Box::new(stream))
    }
}
