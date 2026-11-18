use std::error::Error as StdError;
use std::fmt::Display;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use betterlite_bedrock::aws_settings::AuthParams;
use betterlite_bedrock::realtime_events::failure::ProviderStreamError;
use betterlite_bedrock::realtime_events::scope::EventScope;
use betterlite_bedrock::realtime_transport::error::RealtimeTransportError;
use betterlite_bedrock::realtime_transport::session::{RealtimeCall, RealtimeTransport};
use betterlite_bedrock::signing::target::RegionInputs;
use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_core::api::realtime_session_state::logging::RealtimeSuccessQueue;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

mod client;
mod fakes;
mod runtime;
mod streams;
mod tests;

use client::{AcceptingQueue, FakeLogging, FakeWebSocket};
use fakes::{FakeCredentials, FixedIds};
use runtime::{FakeRuntime, Plan, Recorded};

const TRACE_ID: &str = "trace-bedrock-transport-1";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    runs: Vec<Run>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Run {
    model: String,
    pending: Option<String>,
    frames: Vec<String>,
    chunks: Vec<String>,
    block: Block,
    receive_failure: Option<usize>,
    send_failure: Option<usize>,
    open_error: bool,
    client_send_failure: Option<usize>,
    credentials: CredentialMode,
    auth: Option<Auth>,
    api_base: Option<String>,
    endpoint: Option<String>,
    region: Option<String>,
    committed: bool,
    failure: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Auth {
    access_key_id: String,
    secret_access_key: String,
    session_token: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CredentialMode {
    Resolved,
    Absent,
    Error,
}

// One direction stays pending so the other is always the task that finishes first
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Block {
    None,
    Client,
    Stream,
}

#[derive(Debug, Serialize)]
struct Runs {
    runs: Vec<ScenarioView>,
}

#[derive(Debug, Serialize)]
struct ScenarioView {
    endpoint: Option<String>,
    region: Option<String>,
    access_key_id: Option<String>,
    secret_access_key: Option<String>,
    session_token: Option<String>,
    model_ids: Vec<String>,
    client_types: Vec<String>,
    bedrock_keys: Vec<String>,
    client_close: Option<u16>,
    runtime_closed: bool,
    input_closed: bool,
    resolver_static: bool,
    transport_injected: bool,
    committed: bool,
    failure_set: bool,
    success_flag: bool,
    outcome: &'static str,
    error_kind: Option<&'static str>,
    status_code: Option<u16>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(failure)?;
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(failure)?;
        let views = runtime.block_on(runs(&input.runs, &redactor));
        serde_json::to_value(Runs { runs: views }).map_err(failure)
    }
}

fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "RealtimeTransport".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

async fn runs(runs: &[Run], redactor: &Redactor) -> Vec<ScenarioView> {
    let mut views = Vec::with_capacity(runs.len());
    for run in runs {
        views.push(scenario(run, redactor).await);
    }
    views
}

fn scope_for(run: &Run) -> EventScope {
    let mut scope = EventScope::default();
    if run.committed {
        scope.session_committed = Json::Bool(true);
    }
    if let Some(failure) = &run.failure {
        scope.committed_failure = Json::Text(failure.clone());
    }
    if let Some(pending) = &run.pending {
        scope.pending_session_update = Json::Text(pending.clone());
    }
    scope
}

fn auth_params(auth: Option<&Auth>) -> AuthParams {
    AuthParams {
        aws_access_key_id: auth.map(|value| SecretString::from(value.access_key_id.clone())),
        aws_secret_access_key: auth
            .map(|value| SecretString::from(value.secret_access_key.clone())),
        aws_session_token: auth
            .and_then(|value| value.session_token.clone())
            .map(SecretString::from),
        aws_session_name: None,
        aws_profile_name: None,
        aws_role_name: None,
        aws_web_identity_token: None,
        aws_sts_endpoint: None,
        aws_external_id: None,
        aws_session_tags: None,
    }
}

async fn scenario(run: &Run, redactor: &Redactor) -> ScenarioView {
    let recorded = Arc::new(Recorded::default());
    let runtime = FakeRuntime {
        recorded: Arc::clone(&recorded),
        plan: Plan::from_run(run),
    };
    let credentials = FakeCredentials {
        mode: run.credentials,
        auth: run.auth.clone(),
    };
    let client = FakeWebSocket::new(run);
    let logging = FakeLogging::default();
    let accepting_queue = AcceptingQueue;
    let queue: &dyn RealtimeSuccessQueue = &accepting_queue;
    let ids = FixedIds(AtomicU64::new(1));
    let logged_types = LoggedEventTypes::Default;
    let mut scope = scope_for(run);
    let outcome = RealtimeTransport::new(&runtime, &credentials)
        .realtime(RealtimeCall {
            model: &run.model,
            client: &client,
            logging: &logging,
            logging_worker: Some(queue),
            api_base: run.api_base.as_deref(),
            aws_bedrock_runtime_endpoint: run.endpoint.as_deref(),
            aws_region_name: run.region.as_deref(),
            auth: auth_params(run.auth.as_ref()),
            region_inputs: RegionInputs::default(),
            scope: &mut scope,
            ids: &ids,
            redactor,
            logged_types: &logged_types,
            trace_id: TRACE_ID,
        })
        .await;
    let (outcome_label, error_kind, status_code) = match &outcome {
        Ok(()) => ("ok", None, None),
        Err(error) => {
            let (kind, status) = classify(error);
            ("error", Some(kind), status)
        }
    };
    let view = recorded.view();
    let (access_key_id, secret_access_key, session_token) = recorded.identity();
    let configured = view.configured;
    ScenarioView {
        endpoint: view.endpoint,
        region: view.region,
        access_key_id,
        secret_access_key,
        session_token,
        model_ids: view.model_ids,
        client_types: client.sent().iter().map(|text| event_type(text)).collect(),
        bedrock_keys: recorded.sent_to_bedrock().iter().map(bedrock_key).collect(),
        client_close: client.close_code(),
        runtime_closed: recorded.runtime_closed(),
        input_closed: recorded.input_closed(),
        // Both are set at the one seam where the frozen identity and the injected runtime meet
        resolver_static: configured,
        transport_injected: configured,
        committed: matches!(scope.session_committed, Json::Bool(true)),
        failure_set: !matches!(scope.committed_failure, Json::Null),
        success_flag: logging.success_logged(),
        outcome: outcome_label,
        error_kind,
        status_code,
    }
}

fn classify<E, C>(error: &RealtimeTransportError<E, C>) -> (&'static str, Option<u16>)
where
    E: StdError + 'static,
    C: StdError + 'static,
{
    match error {
        RealtimeTransportError::Committed(committed) => ("committed", Some(committed.status_code)),
        RealtimeTransportError::NoCredentials { status_code, .. } => {
            ("no_credentials", Some(*status_code))
        }
        RealtimeTransportError::Provider(ProviderStreamError::Bedrock { status_code, .. }) => {
            ("provider_bedrock", Some(*status_code))
        }
        RealtimeTransportError::Provider(ProviderStreamError::Unchanged(_)) => {
            ("provider_unchanged", None)
        }
        RealtimeTransportError::Stream(_) => ("stream", None),
        RealtimeTransportError::Client(_) => ("client", None),
        RealtimeTransportError::Credential(_) => ("credential", None),
        RealtimeTransportError::Endpoint(_) => ("endpoint", None),
        RealtimeTransportError::SessionCreated { .. }
        | RealtimeTransportError::Completion(_)
        | RealtimeTransportError::Logging { .. } => ("local", None),
    }
}

fn event_type(message: &str) -> String {
    let Ok(Json::Object(fields)) = serde_json::from_str::<Json>(message) else {
        return String::new();
    };
    match fields.get("type") {
        Some(Json::Text(text)) => text.clone(),
        _ => String::new(),
    }
}

fn bedrock_key(frame: &Json) -> String {
    let Json::Object(fields) = frame else {
        return String::new();
    };
    let Some(Json::Object(event)) = fields.get("event") else {
        return String::new();
    };
    event.keys().next().cloned().unwrap_or_default()
}
