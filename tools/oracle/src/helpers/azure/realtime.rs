use std::collections::BTreeMap;

use betterlite_azure::realtime::connect::{RealtimeConnectSettings, prepare_connection};
use betterlite_azure::realtime::failure::{ClientAction, RealtimeFailure, client_action};
use betterlite_azure::realtime::http_config::{
    RealtimeHttpSettings, realtime_calls_headers, validate_environment,
};
use betterlite_azure::realtime::protocol::client_protocol;
use betterlite_config::tls::Verification;
use betterlite_core::api::realtime_contracts::close::close_after_handshake_refusal;
use betterlite_core::api::realtime_contracts::transport::WebSocketScope;
use betterlite_openai::openai_base_config::Headers;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use self::probe::{
    RecordingSocket, beta_from_scope, closed_json, helper_error, runtime, send_and_close, sent_json,
};

mod probe;
mod tests;

pub(crate) struct Op;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Connect,
    Protocol,
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
    api_version: Option<String>,
    realtime_protocol: Option<String>,
    query_params: Option<BTreeMap<String, Option<String>>>,
    api_key: Option<String>,
    azure_ad_token: Option<String>,
}

#[derive(Deserialize)]
struct ProtocolSetup {
    configured_protocol: Option<String>,
    query_params: Option<BTreeMap<String, Option<String>>>,
}

#[derive(Deserialize)]
struct FailureSetup {
    status_code: u16,
    send_error: Option<String>,
    close_error: Option<String>,
}

#[derive(Deserialize)]
struct HttpSetup {
    api_base: Option<String>,
    api_key: Option<String>,
    api_version: Option<String>,
    litellm_api_base: Option<String>,
    litellm_api_key: Option<String>,
    ephemeral_key: String,
    headers: Headers,
}

#[derive(Deserialize)]
pub(crate) struct Input {
    kind: Kind,
    model: String,
    scope_headers: Vec<(String, String)>,
    ssl_verification: SslSetting,
    max_message_size_bytes: usize,
    azure_api_base: Option<String>,
    azure_api_key: Option<String>,
    azure_api_version: Option<String>,
    queries: Vec<QuerySetup>,
    protocols: Vec<ProtocolSetup>,
    failures: Vec<FailureSetup>,
    http_setups: Vec<HttpSetup>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.kind {
            Kind::Connect => connect_answer(input),
            Kind::Protocol => Ok(protocol_answer(input)),
            Kind::Handshake | Kind::Error => failure_answer(input),
            Kind::Http => Ok(json!({
                "http": input
                    .http_setups
                    .iter()
                    .map(|setup| http_view(input, setup))
                    .collect::<Vec<_>>()
            })),
        }
    }
}

fn connect_view(input: &Input, setup: &QuerySetup) -> Result<Value, Box<HelperError>> {
    let params = setup
        .query_params
        .as_ref()
        .map(|map| map.clone().into_iter().collect());
    let api_key = setup.api_key.as_deref().map(SecretString::from);
    let ad_token = setup.azure_ad_token.as_deref().map(SecretString::from);
    let verification = input.ssl_verification.verification();
    let plan = prepare_connection(&RealtimeConnectSettings {
        api_base: setup.api_base.as_deref(),
        api_key: api_key.as_ref(),
        azure_ad_token: ad_token.as_ref(),
        model: &input.model,
        api_version: setup.api_version.as_deref(),
        realtime_protocol: setup.realtime_protocol.as_deref(),
        query_params: params.as_ref(),
        ssl_verification: &verification,
        max_message_size_bytes: input.max_message_size_bytes,
    })
    .map_err(|error| helper_error(&error.to_string()))?;
    Ok(json!({
        "url": plan.url,
        "headers": plan.headers.iter().map(|(name, value)| (name.clone(), value.clone())).collect::<Vec<_>>(),
        "max_size": plan.max_message_size_bytes,
        "ssl": plan.ssl,
    }))
}

fn connect_answer(input: &Input) -> HelperResult {
    let mut connects = Vec::with_capacity(input.queries.len());
    for setup in &input.queries {
        connects.push(connect_view(input, setup)?);
    }
    Ok(json!({"connects": connects}))
}

fn protocol_answer(input: &Input) -> Value {
    let sent_beta = beta_from_scope(&input.scope_headers);
    let protocols = input
        .protocols
        .iter()
        .map(|setup| {
            let params = setup
                .query_params
                .as_ref()
                .map(|map| map.clone().into_iter().collect());
            client_protocol(
                setup.configured_protocol.as_deref(),
                params.as_ref(),
                sent_beta,
            )
        })
        .collect::<Vec<_>>();
    json!({"protocols": protocols})
}

const fn failure_action(kind: Kind, failure: &FailureSetup) -> RealtimeFailure {
    match kind {
        Kind::Handshake => RealtimeFailure::Handshake {
            status_code: failure.status_code,
        },
        Kind::Connect | Kind::Protocol | Kind::Error | Kind::Http => RealtimeFailure::Other,
    }
}

fn failure_view(
    input: &Input,
    failure: &FailureSetup,
    executor: &Runtime,
) -> Result<Value, Box<HelperError>> {
    let mut socket = RecordingSocket {
        scope: WebSocketScope {
            headers: input.scope_headers.clone(),
        },
        sent: Vec::new(),
        closed: None,
        send_error: failure.send_error.clone(),
        close_error: failure.close_error.clone(),
    };
    let action = client_action(failure_action(input.kind, failure));
    match action {
        ClientAction::RefuseHandshake { status_code } => {
            executor
                .block_on(close_after_handshake_refusal(&mut socket, status_code))
                .map_err(|error| helper_error(&error.to_string()))?;
        }
        ClientAction::SendErrorAndClose {
            error_type,
            message,
            close,
        } => {
            executor
                .block_on(send_and_close(&mut socket, error_type, message, &close))
                .map_err(|error| helper_error(&error.to_string()))?;
        }
    }
    Ok(json!({
        "sent": sent_json(&socket)?,
        "closed": closed_json(&socket),
        "send_raised": socket.send_error.is_some(),
        "close_raised": socket.close_error.is_some(),
    }))
}

fn failure_answer(input: &Input) -> HelperResult {
    let executor = runtime()?;
    let mut failures = Vec::with_capacity(input.failures.len());
    for failure in &input.failures {
        failures.push(failure_view(input, failure, &executor)?);
    }
    Ok(json!({"failures": failures}))
}

fn http_view(input: &Input, setup: &HttpSetup) -> Value {
    let settings = RealtimeHttpSettings {
        litellm_api_base: setup.litellm_api_base.clone(),
        litellm_api_key: setup.litellm_api_key.as_deref().map(SecretString::from),
        azure_api_base: input.azure_api_base.clone(),
        azure_api_key: input.azure_api_key.as_deref().map(SecretString::from),
        azure_api_version: input.azure_api_version.clone(),
    };
    let base = settings.api_base(setup.api_base.as_deref());
    let key = settings.api_key(setup.api_key.as_deref());
    let headers = validate_environment(&setup.headers, Some(&key));
    json!({
        "api_base": base,
        "api_key": key.expose_secret(),
        "complete_url": settings.complete_url(setup.api_base.as_deref(), setup.api_version.as_deref()),
        "calls_url": settings.calls_url(setup.api_base.as_deref(), setup.api_version.as_deref()),
        "transcription_url": settings.transcription_session_url(setup.api_base.as_deref(), setup.api_version.as_deref()),
        "headers": headers,
        "calls_headers": realtime_calls_headers(&setup.ephemeral_key),
    })
}
