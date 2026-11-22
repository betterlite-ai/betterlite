use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_config::tls::Verification;
use betterlite_core::api::realtime_backend_transport::error::{
    ClientCloseError, RealtimeConnectionError,
};
use betterlite_core::api::realtime_backend_transport::protocol::{
    ClientHandle, RealtimeClientSocket, RealtimeSessionRelay, SessionRelay,
};
use betterlite_core::api::realtime_backend_transport::session::RealtimeCall;
use betterlite_core::api::realtime_contracts::error::HeaderMap;
use betterlite_core::api::realtime_contracts::transport::WebSocketScope;
use betterlite_core::api::realtime_guardrail_policy::RealtimeGuardrailCallback;
use betterlite_core::api::request_destination::RequestData;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use serde_json::{Value, json};
use tokio::runtime::Builder;
use tokio::sync::Mutex as TokioMutex;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use super::input::{Input, ProviderDocument};
use super::provider::{ScriptedError, ScriptedProvider};
use super::transport::{
    CloseRecord, DialRecord, Frames, Locked as _, ScriptedClient, ScriptedDialer, ssl_name_of,
};
use super::{Failure, failure};

const DEADLINE: Duration = Duration::from_secs(120);

struct NoCallbacks;

impl RealtimeGuardrailCallback for NoCallbacks {
    type Error = Infallible;

    fn is_custom_guardrail(&self) -> bool {
        false
    }

    fn should_run_guardrail(
        &self,
        _data: &RequestData,
        _event: GuardrailEventHooks,
    ) -> Result<bool, Infallible> {
        Ok(false)
    }
}

// The Python session ends as soon as its client socket disconnects, before it reads or writes the backend, so the relay returns at once
struct ScriptedRelay;

#[async_trait]
impl RealtimeSessionRelay for ScriptedRelay {
    type Error = Infallible;

    async fn relay(&mut self, _session: SessionRelay<'_>) -> Result<(), Infallible> {
        Ok(())
    }
}

struct Probe {
    records: Arc<Mutex<Vec<DialRecord>>>,
    backend_sent: Frames,
    backend_closed: Arc<Mutex<bool>>,
    client_sent: Frames,
    client_close: Arc<Mutex<Option<CloseRecord>>>,
}

impl Probe {
    fn new() -> Self {
        Self {
            records: Arc::new(Mutex::new(Vec::new())),
            backend_sent: Arc::new(Mutex::new(Vec::new())),
            backend_closed: Arc::new(Mutex::new(false)),
            client_sent: Arc::new(Mutex::new(Vec::new())),
            client_close: Arc::new(Mutex::new(None)),
        }
    }

    fn client(&self, input: &Input) -> ClientHandle {
        let socket: Box<dyn RealtimeClientSocket> = Box::new(ScriptedClient {
            sent: Arc::clone(&self.client_sent),
            closed: Arc::clone(&self.client_close),
            send_error: input.send_error.clone(),
            close_error: input.close_error.clone(),
        });
        ClientHandle {
            socket: Arc::new(TokioMutex::new(socket)),
            scope: WebSocketScope::default(),
        }
    }

    fn snapshot(&self) -> Value {
        let dials: Vec<Value> = self.records.locked().iter().map(dial_value).collect();
        let backend: Vec<Value> = self
            .backend_sent
            .locked()
            .iter()
            .map(|text| frame(text.as_str()))
            .collect();
        let client: Vec<Value> = self
            .client_sent
            .locked()
            .iter()
            .map(|text| frame(text.as_str()))
            .collect();
        json!({
            "dials": dials,
            "backend": backend,
            "backend_closed": *self.backend_closed.locked(),
            "client": client,
            "client_close": close_value(self.client_close.locked().as_ref()),
        })
    }
}

fn dial_value(record: &DialRecord) -> Value {
    let headers: Vec<Value> = record
        .headers
        .iter()
        .map(|(name, value)| json!([name, value]))
        .collect();
    json!({
        "url": record.url,
        "headers": headers,
        "ssl": ssl_name_of(&record.ssl),
        "max_size": record.max_size,
        "open_timeout": record.open_timeout.as_secs_f64(),
    })
}

fn close_value(close: Option<&CloseRecord>) -> Value {
    match close {
        Some((code, reason)) => json!({"code": code, "reason": reason}),
        None => Value::Null,
    }
}

// A frame is compared as the JSON it encodes, because each side re-encodes a rewritten frame its own way
fn frame(text: &str) -> Value {
    serde_json::from_str::<Json>(text).map_or_else(
        |_error| json!({"kind": "text", "text": text}),
        |value| json!({"kind": "json", "value": value}),
    )
}

fn close_failure(input: &Input, error: &ClientCloseError) -> Failure {
    let class = input
        .close_error
        .as_ref()
        .map_or("Exception", |document| document.error_class.as_str());
    failure(class, error.to_string())
}

fn connection_failure(
    input: &Input,
    error: RealtimeConnectionError<ScriptedError, Infallible, Infallible>,
) -> Failure {
    match error {
        RealtimeConnectionError::Config(source) => failure(&source.class, source.message),
        RealtimeConnectionError::Close(source) => close_failure(input, &source),
        RealtimeConnectionError::UnexpectedClose { close_error } => failure(
            "Exception",
            format!("Unexpected error while closing WebSocket: {close_error}"),
        ),
        RealtimeConnectionError::Dial(source) => failure("Exception", source.to_string()),
        RealtimeConnectionError::Guardrail(source) => failure("Exception", source.to_string()),
        RealtimeConnectionError::Session(source) => failure("Exception", source.to_string()),
        RealtimeConnectionError::Contract(source) => failure("Exception", source.to_string()),
        RealtimeConnectionError::Relay(source) => failure("Exception", source.to_string()),
        RealtimeConnectionError::Backend(source) | RealtimeConnectionError::Client(source) => {
            failure("Exception", source.to_string())
        }
    }
}

fn build_headers(input: &Input) -> HeaderMap {
    input
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect()
}

struct CallContext<'call> {
    headers: &'call HeaderMap,
    redactor: &'call Redactor,
    probe: &'call Probe,
    dialer: &'call ScriptedDialer,
}

fn query_params_of(input: &Input) -> Option<IndexMap<String, String>> {
    input
        .intent
        .as_ref()
        .map(|intent| IndexMap::from([(String::from("intent"), intent.clone())]))
}

const fn verification_of(input: &Input) -> Verification {
    if input.ssl_verify {
        Verification::Enabled
    } else {
        Verification::Disabled
    }
}

async fn observe(input: &Input) -> HelperResult {
    let Some(document) = input.provider.clone() else {
        return Err(failure(
            "ValueError",
            String::from("the connection input names a provider"),
        ));
    };
    let headers = build_headers(input);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
        .map_err(|error| failure("ValueError", error.to_string()))?;
    let probe = Probe::new();
    let dialer = ScriptedDialer {
        attempts: Mutex::new(input.dials.iter().cloned().collect()),
        records: Arc::clone(&probe.records),
        sent: Arc::clone(&probe.backend_sent),
        closed: Arc::clone(&probe.backend_closed),
    };
    let context = CallContext {
        headers: &headers,
        redactor: &redactor,
        probe: &probe,
        dialer: &dialer,
    };
    if let Err(error) = call_of(input, document, &context).await {
        return Err(connection_failure(input, error));
    }
    Ok(probe.snapshot())
}

async fn call_of(
    input: &Input,
    document: ProviderDocument,
    context: &CallContext<'_>,
) -> Result<(), RealtimeConnectionError<ScriptedError, Infallible, Infallible>> {
    let callbacks: [&NoCallbacks; 0] = [];
    let mut relay = ScriptedRelay;
    let query_params = query_params_of(input);
    RealtimeCall {
        model: &input.model,
        api_base: input.api_base.as_deref(),
        api_key: input.api_key.as_deref(),
        headers: context.headers,
        provider: Arc::new(ScriptedProvider {
            document,
            url: input.url.clone(),
        }),
        dialer: context.dialer,
        client: context.probe.client(input),
        relay: &mut relay,
        callbacks: &callbacks,
        logging: None,
        logging_worker: None,
        logging_session_id: "trace-1",
        user_api_key_dict: None,
        logged_real_time_event_types: LoggedEventTypes::Default,
        litellm_metadata: input.metadata.as_ref(),
        query_params: query_params.as_ref(),
        verification: verification_of(input),
        max_message_size_bytes: input.max_message_size_bytes,
        redactor: context.redactor,
        cancellation: CancellationToken::new(),
    }
    .async_realtime()
    .await
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
        .block_on(async { timeout(DEADLINE, observe(input)).await })
        .map_err(|_elapsed| {
            failure(
                "TimeoutError",
                String::from("the helper ran past its deadline"),
            )
        })?
}
