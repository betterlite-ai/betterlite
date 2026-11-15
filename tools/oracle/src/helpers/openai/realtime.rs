use std::collections::BTreeMap;

use betterlite_config::tls::Verification;
use betterlite_core::api::realtime_contracts::close::close_after_handshake_refusal;
use betterlite_core::api::realtime_contracts::transport::WebSocketScope;
use betterlite_openai::realtime::connect::{RealtimeConnectSettings, prepare_connection};
use betterlite_openai::realtime::failure::{
    ClientAction, RealtimeClose, RealtimeFailure, client_action,
};
use betterlite_openai::realtime::headers::OPENAI_BETA_HEADER;
use betterlite_openai::realtime::http_config::{RealtimeHttpSettings, validate_environment};
use betterlite_openai::realtime::url::QueryParams;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

use self::probe::{
    RecordingLogger, RecordingSocket, beta_realtime, closed_json, helper_error, runtime, sent_json,
    socket_headers,
};

mod probe;
mod tests;

pub(crate) struct Op;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Connect,
    Handshake,
    Error,
    Http,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SslSetting {
    Enabled,
    Disabled,
}

impl SslSetting {
    const fn verification(self) -> Verification {
        match self {
            Self::Enabled => Verification::Enabled,
            Self::Disabled => Verification::Disabled,
        }
    }
}

#[derive(Deserialize)]
struct QuerySetup {
    api_base: Option<String>,
    query_params: Option<BTreeMap<String, Option<String>>>,
}

#[derive(Deserialize)]
struct FailureSetup {
    status_code: u16,
    message: String,
    close_error: Option<String>,
}

#[derive(Deserialize)]
struct HttpSetup {
    api_base: Option<String>,
    api_key: Option<String>,
    litellm_api_base: Option<String>,
    litellm_api_key: Option<String>,
    litellm_openai_key: Option<String>,
    headers: BTreeMap<String, String>,
}

#[derive(Deserialize)]
pub(crate) struct Input {
    kind: Kind,
    api_key: Option<String>,
    model: String,
    scope_headers: Vec<(String, String)>,
    ssl_verification: SslSetting,
    max_message_size_bytes: usize,
    openai_api_base: Option<String>,
    openai_api_key: Option<String>,
    queries: Vec<QuerySetup>,
    failures: Vec<FailureSetup>,
    http_setups: Vec<HttpSetup>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.kind {
            Kind::Http => http_answer(input),
            Kind::Connect => connect_answer(input),
            Kind::Handshake | Kind::Error => failure_answer(input),
        }
    }
}

fn query_params(value: &BTreeMap<String, Option<String>>) -> QueryParams {
    value.clone().into_iter().collect()
}

fn query_json(value: &QueryParams) -> Value {
    Value::Object(
        value
            .iter()
            .map(|(name, item)| {
                let shown = item
                    .as_ref()
                    .map_or(Value::Null, |text| Value::String(text.clone()));
                (name.clone(), shown)
            })
            .collect(),
    )
}

fn pre_call_json(call: &probe::RecordedCall) -> Value {
    json!({
        "input": Value::Null,
        "api_base": call.api_base,
        "header_names": call.headers.keys().cloned().collect::<Vec<_>>(),
        "openai_beta": call.headers.get(OPENAI_BETA_HEADER),
        "query_params": query_json(&call.query_params),
    })
}

fn connect_answer(input: &Input) -> HelperResult {
    let verification = input.ssl_verification.verification();
    let beta = beta_realtime(input);
    let mut logger = RecordingLogger::default();
    let mut connects = Vec::with_capacity(input.queries.len());
    for setup in &input.queries {
        let params = setup.query_params.as_ref().map(query_params);
        let api_key = input.api_key.as_deref().map(SecretString::from);
        let start = logger.calls.len();
        let plan = prepare_connection(
            &RealtimeConnectSettings {
                api_base: setup.api_base.as_deref(),
                api_key: api_key.as_ref(),
                model: &input.model,
                query_params: params.as_ref(),
                beta_realtime: beta,
                ssl_verification: &verification,
                max_message_size_bytes: input.max_message_size_bytes,
            },
            &mut logger,
        )
        .map_err(|error| helper_error(&error.to_string()))?;
        let call = logger
            .calls
            .get(start)
            .ok_or_else(|| helper_error("the pre-call log was not recorded"))?;
        connects.push(json!({
            "url": plan.url,
            "headers": plan.headers.iter().map(|(name, value)| (name.clone(), value.clone())).collect::<Vec<_>>(),
            "max_size": plan.max_message_size_bytes,
            "ssl": plan.ssl,
            "force_transcription_model": plan.force_transcription_model,
            "normalizer_none": plan.event_normalizer.is_none(),
            "pre_call": pre_call_json(call),
        }));
    }
    Ok(json!({"connects": connects}))
}

fn failure_action(
    kind: Kind,
    failure: &FailureSetup,
    redactor: &Redactor,
) -> Result<ClientAction, Box<HelperError>> {
    let outcome = match (kind, failure.close_error.as_deref()) {
        (Kind::Handshake, _) => client_action(
            RealtimeFailure::Handshake {
                status_code: failure.status_code,
            },
            redactor,
        ),
        (_, None) => client_action(
            RealtimeFailure::Other {
                message: &failure.message,
            },
            redactor,
        ),
        (_, Some(close_error)) => client_action(
            RealtimeFailure::CloseFailed {
                message: &failure.message,
                close_error,
            },
            redactor,
        ),
    };
    outcome.map_err(|error| helper_error(&error.to_string()))
}

fn failure_frame(action: &ClientAction) -> Option<&RealtimeClose> {
    match action {
        ClientAction::Close(frame) | ClientAction::IgnoreCloseFailure { attempted: frame } => {
            Some(frame)
        }
        ClientAction::RefuseHandshake { .. } => None,
    }
}

fn failure_answer(input: &Input) -> HelperResult {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
        .map_err(|error| helper_error(&error.to_string()))?;
    let runtime = runtime()?;
    let mut failures = Vec::with_capacity(input.failures.len());
    for failure in &input.failures {
        let mut socket = RecordingSocket {
            scope: WebSocketScope {
                headers: socket_headers(input),
            },
            sent: Vec::new(),
            closed: None,
            close_error: failure.close_error.clone(),
        };
        let action = failure_action(input.kind, failure, &redactor)?;
        if let Some(frame) = failure_frame(&action) {
            drop(runtime.block_on(socket.close(frame.code, Some(&frame.reason))));
        }
        if let ClientAction::RefuseHandshake { status_code } = action {
            runtime
                .block_on(close_after_handshake_refusal(&mut socket, status_code))
                .map_err(|error| helper_error(&error.to_string()))?;
        }
        failures.push(json!({
            "sent": sent_json(&socket)?,
            "closed": closed_json(&socket),
            "close_raised": socket.close_error.is_some(),
        }));
    }
    Ok(json!({"failures": failures}))
}

fn http_answer(input: &Input) -> HelperResult {
    let mut http = Vec::with_capacity(input.http_setups.len());
    for setup in &input.http_setups {
        let settings = RealtimeHttpSettings {
            litellm_api_base: setup.litellm_api_base.clone(),
            litellm_api_key: setup.litellm_api_key.as_deref().map(SecretString::from),
            litellm_openai_key: setup.litellm_openai_key.as_deref().map(SecretString::from),
            openai_api_base: input.openai_api_base.clone(),
            openai_api_key: input.openai_api_key.as_deref().map(SecretString::from),
        };
        let api_base = settings.api_base(setup.api_base.as_deref());
        let api_key = settings.api_key(setup.api_key.as_deref());
        let copied = setup.headers.clone();
        let headers = validate_environment(&copied, Some(&api_key));
        http.push(json!({
            "api_base": api_base,
            "api_key": api_key.expose_secret(),
            "complete_url": settings.complete_url(setup.api_base.as_deref()),
            "calls_url": settings.calls_url(setup.api_base.as_deref()),
            "transcription_url": settings.transcription_session_url(setup.api_base.as_deref()),
            "headers": headers,
        }));
    }
    Ok(json!({"http": http}))
}
