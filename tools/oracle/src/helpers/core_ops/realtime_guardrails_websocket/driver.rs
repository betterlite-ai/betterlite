mod step;

use std::future::Future;
use std::mem::take;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_core::api::realtime_contracts::config::{BaseRealtimeConfig, Environment};
use betterlite_core::api::realtime_contracts::error::HeaderMap;
use betterlite_core::api::realtime_contracts::events::{
    RealtimeResponseTransformInput, RealtimeResponseTypedDict,
};
use betterlite_core::api::realtime_contracts::transport::{RealtimeMessage, WebSocketScope};
use betterlite_core::api::realtime_guardrails::callback::{
    GuardrailInput, GuardrailInputType, RealtimeGuardrail,
};
use betterlite_core::api::realtime_guardrails::error::RealtimeGuardrailsError;
use betterlite_core::api::realtime_session_state::{ConnectionSetup, RealtimeStreaming};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::helper::HelperResult;
use serde_json::{Value, json};
use tokio::runtime::Builder;
use tokio::time::timeout;

use super::input::Input;
use super::scripted::{Records, ScriptedError, ScriptedGuardrail};
use super::transport::{Locked as _, ScriptedBackend, ScriptedClient, Timeline, WireLog};
use super::{Failure, failure};

const DEADLINE: Duration = Duration::from_secs(120);

type DriverFuture<'driver, T> = Pin<Box<dyn Future<Output = Result<T, Failure>> + 'driver>>;

// The runner only needs a provider type to be parameterized by; guardrail frames go through the default send path
struct ScriptedConfig;

#[async_trait]
impl BaseRealtimeConfig for ScriptedConfig {
    type Error = ScriptedError;
    type Backend = ScriptedBackend;

    fn validate_environment(
        &self,
        _environment: &Environment<'_>,
    ) -> Result<HeaderMap, ScriptedError> {
        Ok(HeaderMap::new())
    }

    fn transform_realtime_request(
        &self,
        _message: &str,
        _model: &str,
        _session_configuration_request: Option<&str>,
    ) -> Result<Vec<RealtimeMessage>, ScriptedError> {
        Ok(Vec::new())
    }

    fn transform_realtime_response(
        &self,
        _message: &RealtimeMessage,
        _model: &str,
        _input: &RealtimeResponseTransformInput,
    ) -> Result<RealtimeResponseTypedDict, ScriptedError> {
        Err(ScriptedError {
            message: String::from("the scripted provider is never configured"),
        })
    }
}

struct Session {
    state: RealtimeStreaming,
    backend: Arc<Mutex<WireLog>>,
    client: Arc<Mutex<WireLog>>,
    timeline: Timeline,
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
        .block_on(timeout(DEADLINE, observe(input)))
        .map_err(|elapsed| {
            failure(
                "TimeoutError",
                format!("the helper ran past its deadline: {elapsed}"),
            )
        })?
}

fn observe(input: &Input) -> DriverFuture<'_, Value> {
    Box::pin(async move {
        let mut session = open_session(input);
        let records = Arc::new(Mutex::new(Records::default()));
        let guardrails: Vec<ScriptedGuardrail> = input
            .guardrails
            .iter()
            .map(|document| ScriptedGuardrail::new(document, &records))
            .collect();
        let callbacks = callback_list(input, &guardrails)?;
        let mut runs = Vec::new();
        for document in &input.runs {
            let (frame, failed) = session.step(&callbacks, document).await;
            runs.push(frame);
            if failed {
                break;
            }
        }
        let (checks, applied) = recorded_records(&records);
        Ok(report(&session, &runs, &checks, &applied))
    })
}

fn callback_list<'run>(
    input: &Input,
    guardrails: &'run [ScriptedGuardrail],
) -> Result<Vec<&'run dyn RealtimeGuardrail<Error = ScriptedError>>, Failure> {
    let mut callbacks: Vec<&'run dyn RealtimeGuardrail<Error = ScriptedError>> = Vec::new();
    for index in &input.callbacks {
        let guardrail = guardrails.get(*index).ok_or_else(|| {
            failure(
                "ValueError",
                format!("the callback index {index} is past the guardrails"),
            )
        })?;
        callbacks.push(guardrail);
    }
    Ok(callbacks)
}

fn report(session: &Session, runs: &[Value], checks: &[Value], applied: &[Value]) -> Value {
    json!({
        "runs": runs,
        "timeline": session.timeline.locked().clone(),
        "checks": checks,
        "applied": applied,
        "state": {
            "violation_count": session.state.setup.violation_count,
            "backend_closes": session.backend.locked().closes,
            "client_closes": session.client.locked().closes,
        },
    })
}

fn open_session(input: &Input) -> Session {
    let timeline: Timeline = Arc::new(Mutex::new(Vec::new()));
    let backend = Arc::new(Mutex::new(WireLog::new(input.backend_failures.clone())));
    let client = Arc::new(Mutex::new(WireLog::new(input.client_failures.clone())));
    let state = RealtimeStreaming::new(ConnectionSetup {
        client: Some(Box::new(ScriptedClient {
            log: Arc::clone(&client),
            timeline: Arc::clone(&timeline),
            scope: WebSocketScope::default(),
        })),
        backend: Some(Box::new(ScriptedBackend {
            log: Arc::clone(&backend),
            timeline: Arc::clone(&timeline),
            close_failure: input.close_failure,
        })),
        logging: None,
        logging_worker: None,
        provider_config: None,
        event_normalizer: None,
        logged_real_time_event_types: LoggedEventTypes::Default,
        model: String::from("gpt-realtime"),
        user_api_key_dict: input.user_api_key_dict.clone(),
        request_data: Some(input.request_data.clone()),
        force_transcription_model: None,
        client_wants_beta: false,
        backend_uses_beta_protocol: None,
    });
    Session {
        state,
        backend,
        client,
        timeline,
    }
}

// The frames each socket accepted since the last run, recorded the way the Python helper records them
fn frame_log(session: &Session) -> (Vec<Value>, Vec<Value>) {
    let backend = take(&mut session.backend.locked().sent);
    let client = take(&mut session.client.locked().sent);
    let backend_frames: Vec<Value> = backend.iter().map(|text| recorded_frame(text)).collect();
    let client_frames: Vec<Value> = client.iter().map(|text| recorded_frame(text)).collect();
    (backend_frames, client_frames)
}

fn recorded_records(records: &Arc<Mutex<Records>>) -> (Vec<Value>, Vec<Value>) {
    let records = records.locked();
    let checks = records
        .checks
        .iter()
        .map(|(event, data)| json!({"event": event, "data": data}))
        .collect();
    let applied = records.applied.iter().map(recorded_input).collect();
    drop(records);
    (checks, applied)
}

fn recorded_input(input: &GuardrailInput) -> Value {
    let input_type = match input.input_type {
        GuardrailInputType::Request => "request",
        GuardrailInputType::Response => "response",
    };
    json!({
        "inputs": {
            "texts": &input.texts,
            "images": &input.images,
        },
        "request_data": &input.request_data,
        "input_type": input_type,
    })
}

fn recorded_frame(text: &str) -> Value {
    match serde_json::from_str::<Json>(text) {
        Ok(value) => json!({"wire": "text", "payload": {"kind": "json", "value": value}}),
        Err(_error) => json!({"wire": "text", "payload": {"kind": "text", "text": text}}),
    }
}

fn label(error: &RealtimeGuardrailsError<ScriptedError, ScriptedError>) -> &'static str {
    match error {
        RealtimeGuardrailsError::Unexpected(_) => "unexpected",
        RealtimeGuardrailsError::Send(_) => "send",
        RealtimeGuardrailsError::Client(_) => "client",
        RealtimeGuardrailsError::Close(_) => "close",
        RealtimeGuardrailsError::Predicate(_) => "predicate",
        RealtimeGuardrailsError::BackendMissing | RealtimeGuardrailsError::ClientMissing => {
            "socket"
        }
        RealtimeGuardrailsError::Encode(_) => "encode",
    }
}
