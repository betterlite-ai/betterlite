use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_core::api::realtime_contracts::transport::{
    RealtimeEventNormalizer, WebSocketScope,
};
use betterlite_core::api::realtime_guardrail_policy::RealtimeGuardrailCallback;
use betterlite_core::api::realtime_sender::RealtimeSender;
use betterlite_core::api::realtime_sender::error::RealtimeSenderError;
use betterlite_core::api::realtime_session_state::{
    ConnectionSetup, RealtimeProviderConfig, RealtimeStreaming,
};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::helper::HelperResult;
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::Receiver;
use tokio::task::JoinSet;
use tokio::time::timeout;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

use super::Failure;
use super::failure;
use super::input::{Input, MAX_FRAMES, Operation};
use super::provider::{Guard, Normalizer, ProviderView, ScriptError, ScriptedProvider};
use super::transport::{
    ClientSink, Locked as _, Recorded, Timeline, WireBackend, WireLog, recorded, relay,
};

const SETTLE: Duration = Duration::from_secs(10);
const DEADLINE: Duration = Duration::from_secs(120);

pub(crate) type DriverFuture<'driver, T> =
    Pin<Box<dyn Future<Output = Result<T, Failure>> + 'driver>>;

type Callbacks<'driver> = &'driver [&'driver dyn RealtimeGuardrailCallback<Error = ScriptError>];

struct Session {
    state: RealtimeStreaming,
    provider: Option<ScriptedProvider>,
    guards: Vec<Guard>,
    timeline: Timeline,
    backend_log: Arc<Mutex<WireLog>>,
    client_log: Arc<Mutex<WireLog>>,
}

pub(crate) fn drive(input: &Input) -> HelperResult {
    input
        .check()
        .map_err(|message| failure("InvalidInput", message))?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failure("OSError", error.to_string()))?;
    runtime
        .block_on(timeout(DEADLINE, open(input)))
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
        let (sender, mut receiver) = tokio::sync::mpsc::channel(MAX_FRAMES);
        let mut relayed = JoinSet::new();
        relayed.spawn(relay(listener, sender));
        let outcome = run_operations(input, address, &mut receiver).await;
        relayed.abort_all();
        while relayed.join_next().await.is_some() {}
        outcome
    })
}

async fn run_operations(
    input: &Input,
    address: SocketAddr,
    receiver: &mut Receiver<Recorded>,
) -> Result<Value, Failure> {
    let (socket, _) = connect_async(format!("ws://{address}"))
        .await
        .map_err(|error| failure("TransportError", error.to_string()))?;
    let mut streaming = session(input, socket);
    let callbacks = streaming
        .guards
        .iter()
        .map(|guard| guard as &dyn RealtimeGuardrailCallback<Error = ScriptError>)
        .collect::<Vec<_>>();
    let mut results = Vec::new();
    let mut upstream = Vec::new();
    for operation in &input.operations {
        let state = &mut streaming.state;
        let provider = streaming.provider.as_ref();
        results.push(apply(state, provider, &callbacks, operation).await?);
        let delivered = streaming.backend_log.locked().delivered;
        settle(receiver, &mut upstream, delivered).await?;
    }
    Ok(snapshot(&streaming, results, upstream))
}

fn session(input: &Input, socket: WebSocketStream<MaybeTlsStream<TcpStream>>) -> Session {
    let timeline: Timeline = Arc::new(Mutex::new(Vec::new()));
    let provider = input
        .provider
        .as_ref()
        .map(|document| ScriptedProvider::new(document, Arc::clone(&timeline)));
    let backend_log = Arc::new(Mutex::new(WireLog::new(&input.backend_failures)));
    let client_log = Arc::new(Mutex::new(WireLog::new(&input.client_failures)));
    let backend = WireBackend {
        socket,
        log: Arc::clone(&backend_log),
        timeline: Arc::clone(&timeline),
    };
    let client = ClientSink {
        scope: WebSocketScope {
            headers: if input.client_wants_beta {
                vec![(String::from("openai-beta"), String::from("realtime=v1"))]
            } else {
                Vec::new()
            },
        },
        log: Arc::clone(&client_log),
    };
    Session {
        state: RealtimeStreaming::new(connection(input, client, backend)),
        provider,
        guards: Guard::documents(&input.guardrails),
        timeline,
        backend_log,
        client_log,
    }
}

fn connection(input: &Input, client: ClientSink, backend: WireBackend) -> ConnectionSetup {
    ConnectionSetup {
        client: Some(Box::new(client)),
        backend: Some(Box::new(backend)),
        logging: None,
        logging_worker: None,
        provider_config: input.provider.as_ref().map(|document| {
            Box::new(ProviderView {
                requires_session_configuration: document.requires_session_configuration,
            }) as Box<dyn RealtimeProviderConfig>
        }),
        event_normalizer: Some(Box::new(Normalizer) as Box<dyn RealtimeEventNormalizer>),
        logged_real_time_event_types: LoggedEventTypes::Default,
        model: input.model.clone(),
        user_api_key_dict: None,
        request_data: None,
        force_transcription_model: input.force_transcription_model.clone(),
        client_wants_beta: input.client_wants_beta,
        backend_uses_beta_protocol: input.backend_uses_beta_protocol,
    }
}

fn apply<'driver>(
    state: &'driver mut RealtimeStreaming,
    provider: Option<&'driver ScriptedProvider>,
    callbacks: Callbacks<'driver>,
    operation: &'driver Operation,
) -> DriverFuture<'driver, Value> {
    Box::pin(async move {
        let mut sender = RealtimeSender {
            state,
            provider,
            guardrails: callbacks,
        };
        match operation {
            Operation::Send { text } => Ok(outcome(sender.send_to_backend(text).await)),
            Operation::Event { text } => {
                let event = serde_json::from_str::<Json>(text).unwrap_or(Json::Null);
                Ok(outcome(sender.send_event_to_client(&event, text).await))
            }
            Operation::Disable => Ok(outcome(
                sender.maybe_disable_auto_response().await.map(|()| true),
            )),
            Operation::Cache { text } => {
                sender.cache_session_configuration(text);
                Ok(json!({ "ok": true }))
            }
        }
    })
}

fn outcome<ConfigError>(
    result: Result<bool, RealtimeSenderError<ConfigError, ScriptError>>,
) -> Value
where
    ConfigError: std::error::Error + 'static,
{
    match result {
        Ok(sent) => json!({ "ok": sent }),
        Err(error) => {
            let label = match error {
                RealtimeSenderError::Backend { .. } => "backend",
                RealtimeSenderError::Client { .. } => "client",
                RealtimeSenderError::Transform { .. } => "transform",
                RealtimeSenderError::Predicate { .. } => "predicate",
                RealtimeSenderError::SessionUpdateNotObject => "non-object",
                RealtimeSenderError::GuardrailSetup { .. }
                | RealtimeSenderError::Disable { .. }
                | RealtimeSenderError::BackendMissing
                | RealtimeSenderError::ClientMissing
                | RealtimeSenderError::Beta { .. }
                | RealtimeSenderError::SessionUpdateTooDeep
                | RealtimeSenderError::SessionUpdateInvalid { .. }
                | RealtimeSenderError::Encode { .. } => "other",
            };
            json!({ "error": label })
        }
    }
}

fn settle<'driver>(
    receiver: &'driver mut Receiver<Recorded>,
    received: &'driver mut Vec<Recorded>,
    delivered: usize,
) -> DriverFuture<'driver, ()> {
    Box::pin(async move {
        while received.len() < delivered {
            if received.len() >= MAX_FRAMES {
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
            received.push(frame);
        }
        Ok(())
    })
}

fn snapshot(session: &Session, results: Vec<Value>, upstream: Vec<Recorded>) -> Value {
    let request = session
        .state
        .response
        .session_configuration_request
        .as_deref()
        .map(|text| recorded("text", text.as_bytes()).payload);
    let timeline = session.timeline.locked().clone();
    let sent = session
        .client_log
        .locked()
        .sent
        .iter()
        .map(|text| recorded("text", text.as_bytes()))
        .collect::<Vec<_>>();
    json!({
        "results": results, "timeline": timeline, "upstream": upstream, "client": sent,
        "state": {
            "session_configuration_request": request,
            "content_sent_after_setup": session.state.setup.content_sent_after_setup.0,
            "turn_detection_update_sent": session.state.setup.turn_detection_update_sent.0,
            "is_transcription_session": session.state.is_transcription_session.0,
        }
    })
}
