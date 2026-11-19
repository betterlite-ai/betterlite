use std::convert::Infallible;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_core::api::realtime_guardrail_policy::RealtimeGuardrailCallback;
use betterlite_core::api::realtime_sender::RealtimeSender;
use betterlite_core::api::realtime_session_state::{
    ConnectionSetup, RealtimeProviderConfig, RealtimeStreaming,
};
use betterlite_core::api::realtime_setup_buffer::{
    RealtimeSetupBufferError, SetupBuffer as _, SetupFlush as _,
};
use betterlite_oracle_helper_api::helper::HelperResult;
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Builder;
use tokio::sync::mpsc::{Receiver, channel};
use tokio::task::JoinSet;
use tokio::time::timeout;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

use super::Failure;
use super::failure;
use super::input::{Input, MAX_FRAMES, Operation};
use super::provider::{ProviderView, ScriptedProvider};
use super::transport::{Locked as _, Recorded, WireBackend, WireLog, recorded, relay};

const SETTLE: Duration = Duration::from_secs(10);
const DEADLINE: Duration = Duration::from_secs(120);

pub(crate) type DriverFuture<'driver, T> =
    Pin<Box<dyn Future<Output = Result<T, Failure>> + 'driver>>;

type Callbacks<'driver> = &'driver [&'driver dyn RealtimeGuardrailCallback<Error = Infallible>];

struct Session {
    state: RealtimeStreaming,
    provider: Option<ScriptedProvider>,
    log: Arc<Mutex<WireLog>>,
}

pub(crate) fn drive(input: &Input) -> HelperResult {
    input
        .check()
        .map_err(|message| failure("ValueError", message))?;
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failure("OSError", error.to_string()))?;
    runtime
        .block_on(async { timeout(DEADLINE, open(input)).await })
        .map_err(|_elapsed| {
            failure(
                "TimeoutError",
                String::from("the helper ran past its deadline"),
            )
        })?
}

fn open(input: &Input) -> DriverFuture<'_, Value> {
    Box::pin(async move {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|error| failure("OSError", error.to_string()))?;
        let address = listener
            .local_addr()
            .map_err(|error| failure("OSError", error.to_string()))?;
        let (sender, mut receiver) = channel(MAX_FRAMES);
        let mut relayed = JoinSet::new();
        relayed.spawn(relay(listener, sender));
        let outcome = run_operations(input, address, &mut receiver).await;
        relayed.abort_all();
        while relayed.join_next().await.is_some() {}
        outcome
    })
}

fn run_operations<'driver>(
    input: &'driver Input,
    address: SocketAddr,
    receiver: &'driver mut Receiver<Recorded>,
) -> DriverFuture<'driver, Value> {
    Box::pin(async move {
        let (socket, _) = connect_async(format!("ws://{address}"))
            .await
            .map_err(|error| failure("TransportError", error.to_string()))?;
        let mut streaming = session(input, socket);
        let mut results = Vec::new();
        let mut states = Vec::new();
        let mut upstream = Vec::new();
        for operation in &input.operations {
            let state = &mut streaming.state;
            let provider = streaming.provider.as_ref();
            results.push(apply(state, provider, operation).await?);
            let delivered = streaming.log.locked().delivered;
            settle(receiver, &mut upstream, delivered).await?;
            states.push(observe(&streaming, upstream.len()));
        }
        Ok(snapshot(&streaming, &results, &states, &upstream))
    })
}

fn session(input: &Input, socket: WebSocketStream<MaybeTlsStream<TcpStream>>) -> Session {
    let log = Arc::new(Mutex::new(WireLog {
        attempts: 0,
        delivered: 0,
        failures: input.backend_failures.clone(),
    }));
    let provider = input.provider.as_ref().map(ScriptedProvider::new);
    let backend = WireBackend {
        socket,
        log: Arc::clone(&log),
    };
    Session {
        state: RealtimeStreaming::new(connection(input, backend)),
        provider,
        log,
    }
}

fn connection(input: &Input, backend: WireBackend) -> ConnectionSetup {
    ConnectionSetup {
        client: None,
        backend: Some(Box::new(backend)),
        logging: None,
        logging_worker: None,
        provider_config: input.provider.as_ref().map(
            |document| -> Box<dyn RealtimeProviderConfig> {
                Box::new(ProviderView {
                    requires_session_configuration: document.requires_session_configuration,
                })
            },
        ),
        event_normalizer: None,
        logged_real_time_event_types: LoggedEventTypes::Default,
        model: String::from("scripted"),
        user_api_key_dict: None,
        request_data: None,
        force_transcription_model: None,
        client_wants_beta: false,
        backend_uses_beta_protocol: None,
    }
}

fn apply<'driver>(
    state: &'driver mut RealtimeStreaming,
    provider: Option<&'driver ScriptedProvider>,
    operation: &'driver Operation,
) -> DriverFuture<'driver, Value> {
    Box::pin(async move {
        match operation {
            Operation::Deferred => Ok(json!({ "ok": state.uses_deferred_backend_setup() })),
            Operation::ShouldBuffer { text } => {
                Ok(judgement(state.should_buffer_until_setup(text)))
            }
            Operation::Buffer { text } => {
                Ok(judgement(state.buffer_pending_message(text).map(|()| true)))
            }
            Operation::Complete { value } => {
                state.setup.backend_setup_complete.0 = *value;
                Ok(json!({ "ok": true }))
            }
            Operation::Flushing { value } => {
                state.setup.flushing_pending_messages.0 = *value;
                Ok(json!({ "ok": true }))
            }
            Operation::Flush => flush(state, provider, None).await,
            Operation::FlushWithArrival { text } => {
                flush(state, provider, Some(text.as_str())).await
            }
        }
    })
}

// The Python relay buffers the arrival while the failing send is in flight; here it joins a queue the failure already restored
async fn flush(
    state: &mut RealtimeStreaming,
    provider: Option<&ScriptedProvider>,
    arrival: Option<&str>,
) -> Result<Value, Failure> {
    let callbacks: Callbacks<'_> = &[];
    let mut sender = RealtimeSender {
        state,
        provider,
        guardrails: callbacks,
    };
    let result = sender.flush_pending_messages().await;
    if let Some(text) = arrival {
        sender
            .state
            .buffer_pending_message(text)
            .map_err(|error| failure("ValueError", error.to_string()))?;
    }
    Ok(judgement(result))
}

fn judgement(result: Result<bool, RealtimeSetupBufferError>) -> Value {
    match result {
        Ok(flag) => json!({ "ok": flag }),
        Err(error) => error_outcome(&error),
    }
}

fn error_outcome(error: &RealtimeSetupBufferError) -> Value {
    match error {
        RealtimeSetupBufferError::NotAnObject => json!({ "error": "non-object" }),
        RealtimeSetupBufferError::Decode(_) => json!({ "error": "decode" }),
    }
}

fn observe(session: &Session, upstream: usize) -> Value {
    json!({
        "deferred": session.state.uses_deferred_backend_setup(),
        "setup_complete": session.state.setup.backend_setup_complete.0,
        "flushing": session.state.setup.flushing_pending_messages.0,
        "pending_count": session.state.setup.pending_messages_until_setup.len(),
        "pending_bytes": session.state.setup.pending_messages_byte_total,
        "upstream": upstream,
    })
}

async fn settle(
    receiver: &mut Receiver<Recorded>,
    upstream: &mut Vec<Recorded>,
    delivered: usize,
) -> Result<(), Failure> {
    while upstream.len() < delivered {
        if upstream.len() >= MAX_FRAMES {
            return Err(failure(
                "InvalidInput",
                String::from("the upstream capture is full"),
            ));
        }
        let next = timeout(SETTLE, receiver.recv()).await.map_err(|_elapsed| {
            failure("TimeoutError", String::from("no upstream frame arrived"))
        })?;
        let frame =
            next.ok_or_else(|| failure("TransportError", String::from("upstream ended")))?;
        upstream.push(frame);
    }
    Ok(())
}

fn snapshot(
    session: &Session,
    results: &[Value],
    states: &[Value],
    upstream: &[Recorded],
) -> Value {
    let pending = session
        .state
        .setup
        .pending_messages_until_setup
        .iter()
        .map(|message| recorded("text", message).payload)
        .collect::<Vec<_>>();
    json!({
        "results": results, "states": states, "pending": pending, "upstream": upstream,
    })
}
