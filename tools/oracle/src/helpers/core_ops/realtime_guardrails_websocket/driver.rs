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
use betterlite_core::api::realtime_guardrails::runner::{
    RealtimeGuardrails, RunRealtimeGuardrails,
};
use betterlite_core::api::realtime_session_state::{ConnectionSetup, RealtimeStreaming};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::helper::HelperResult;
use serde_json::{Value, json};
use tokio::runtime::Builder;
use tokio::time::timeout;

use super::input::{Input, RunDocument};
use super::scripted::{Records, ScriptedError, ScriptedGuardrail};
use super::transport::{Locked as _, ScriptedBackend, ScriptedClient, Timeline, WireLog};
use super::{Failure, failure};

const DEADLINE: Duration = Duration::from_secs(120);

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

async fn observe(input: &Input) -> Result<Value, Failure> {
    let mut session = open_session(input);
    let records = Arc::new(Mutex::new(Records::default()));
    let guardrails: Vec<ScriptedGuardrail> = input
        .guardrails
        .iter()
        .map(|document| ScriptedGuardrail::new(document, &records))
        .collect();
    let callbacks = callback_list(input, &guardrails)?;
    let runs = run_all(&mut session, &callbacks, &input.runs).await;
    let (checks, applied) = recorded_records(&records);
    Ok(report(&session, runs, checks, applied))
}

fn callback_list<'run>(
    input: &Input,
    guardrails: &'run [ScriptedGuardrail],
) -> Result<Vec<&'run dyn RealtimeGuardrail<Error = ScriptedError>>, Failure> {
    let mut callbacks = Vec::new();
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

async fn run_all(
    session: &mut Session,
    callbacks: &[&dyn RealtimeGuardrail<Error = ScriptedError>],
    runs: &[RunDocument],
) -> Vec<Value> {
    let mut outcomes = Vec::new();
    for document in runs {
        let outcome = {
            let mut runner: RealtimeGuardrails<'_, ScriptedConfig, ScriptedError> =
                RealtimeGuardrails {
                    state: &mut session.state,
                    provider: None,
                    guardrails: callbacks,
                };
            runner
                .run_realtime_guardrails(
                    &document.transcript,
                    document.item_id.as_deref(),
                    document.pre_block_backend_message.as_deref(),
                    document.event_hooks.as_deref(),
                )
                .await
        };
        let (backend, client) = frame_log(session);
        match outcome {
            Ok(blocked) => outcomes.push(json!({
                "blocked": blocked,
                "error": Value::Null,
                "backend": backend,
                "client": client,
            })),
            Err(error) => {
                outcomes.push(json!({
                    "blocked": Value::Null,
                    "error": label(&error),
                    "backend": backend,
                    "client": client,
                }));
                break;
            }
        }
    }
    outcomes
}

fn report(session: &Session, runs: Vec<Value>, checks: Vec<Value>, applied: Vec<Value>) -> Value {
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
    let backend = std::mem::take(&mut session.backend.locked().sent);
    let client = std::mem::take(&mut session.client.locked().sent);
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
