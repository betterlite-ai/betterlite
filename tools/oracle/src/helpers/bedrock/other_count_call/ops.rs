use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::count_call::counter::{
    BedrockTokenCounter, CountTokensCall, TokenCountResponse, TokenCounter as _,
};
use betterlite_bedrock::count_call::factory::BedrockModelInfo;
use betterlite_bedrock::count_call::handler::{
    BedrockCountTokensHandler, CountTokensCredentials, CountTokensEnvironment, CountTokensHandler,
    CountTokensHandlerError, CountTokensHandlerRequest, CountTokensTransport,
};
use betterlite_bedrock::count_call::reply::CountResponse;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use http::Method;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use secrecy::SecretString;
use serde::Serialize;
use serde_json::json;
use tokio::runtime::{Builder, Runtime};
use tracing::Level;

use crate::helpers::{HelperError, OrHelperError as _};

use super::input::{Call, Environment, Handler, Reply};
use super::logs::Captured;

const REPORTED_HEADERS: [&str; 5] = [
    "content-type",
    "authorization",
    "x-amz-date",
    "x-amz-security-token",
    "x-amz-content-sha256",
];

pub(super) fn run_call(call: &Call, factory: bool) -> Result<Json, Box<HelperError>> {
    let (captured, guard) = super::logs::capture();
    let resolved = Arc::new(AtomicUsize::new(0));
    let transport = Script::new(&call.reply);
    let observed = transport.clone();
    let environment = environment_of(&call.environment);
    let selection = BedrockTokenCounter {
        handler: EmptyHandler,
    };
    let selected = selection.should_use_token_counting_api(call.provider.as_deref());
    let request = CountTokensCall {
        model_to_use: &call.model,
        messages: call.messages.as_ref(),
        deployment: call.deployment.as_ref(),
        request_model: &call.request_model,
        tools: call.tools.as_ref(),
        system: call.system.as_ref(),
        now: epoch(call.now)?,
    };
    let outcome = runtime()?.block_on(count(
        call,
        factory,
        transport,
        &environment,
        &resolved,
        &request,
    ));
    drop(guard);
    as_json(json!({
        "selected": selected,
        "result": outcome.map(|result| result_json(&result)).transpose()?,
        "request": observed.observed()?,
        "debug": debug_events(&captured)?,
        "errors": error_events(&captured)?,
        "warnings": warning_events(&captured)?,
        "resolved": resolved.load(Ordering::Relaxed),
    }))
}

async fn count(
    call: &Call,
    factory: bool,
    transport: Script,
    environment: &CountTokensEnvironment,
    resolved: &Arc<AtomicUsize>,
    request: &CountTokensCall<'_>,
) -> Option<TokenCountResponse> {
    let credentials = FixtureCredentials {
        resolved: Arc::clone(resolved),
    };
    match (&call.handler, factory) {
        (Handler::None, _) => {
            BedrockTokenCounter {
                handler: EmptyHandler,
            }
            .count_tokens(request)
            .await
        }
        (Handler::EmptyObject, _) => {
            BedrockTokenCounter {
                handler: EmptyObjectHandler,
            }
            .count_tokens(request)
            .await
        }
        (Handler::Real, false) => {
            BedrockTokenCounter {
                handler: BedrockCountTokensHandler {
                    transport,
                    credentials,
                    environment: environment.clone(),
                },
            }
            .count_tokens(request)
            .await
        }
        (Handler::Real, true) => {
            BedrockModelInfo {
                environment: environment.clone(),
            }
            .get_token_counter(transport, credentials)
            .count_tokens(request)
            .await
        }
    }
}

fn as_json<T>(value: T) -> Result<Json, Box<HelperError>>
where
    T: Serialize,
{
    let encoded = serde_json::to_value(value).or_value_error()?;
    let decoded = serde_json::from_value(encoded).or_value_error()?;
    Ok(decoded)
}

fn result_json(result: &TokenCountResponse) -> Result<Json, Box<HelperError>> {
    as_json(json!({
        "total_tokens": result.total_tokens,
        "request_model": result.request_model,
        "model_used": result.model_used,
        "tokenizer_type": result.tokenizer_type,
        "original_response": result.original_response,
        "error": result.error,
        "error_message": error_message_kind(result.error_message.as_deref(), result.status_code),
        "status_code": result.status_code,
    }))
}

fn error_message_kind(message: Option<&str>, status: Option<u16>) -> Option<&str> {
    let message = message?;
    match status {
        Some(500) if message.starts_with("CountTokens processing error: ") => Some("processing"),
        Some(500) => Some("generic"),
        Some(_) | None => Some(message),
    }
}

fn debug_events(captured: &Captured) -> Result<Vec<Json>, Box<HelperError>> {
    captured
        .entries(Level::DEBUG)
        .iter()
        .map(|entry| {
            as_json(json!({
                "subject": entry.subject,
                "value": entry
                    .value
                    .as_deref()
                    .map_or(Json::Null, |text| serde_json::from_str(text).unwrap_or(Json::Null)),
            }))
        })
        .collect()
}

fn error_events(captured: &Captured) -> Result<Vec<Json>, Box<HelperError>> {
    captured
        .entries(Level::ERROR)
        .iter()
        .map(|entry| as_json(json!({"subject": entry.subject})))
        .collect()
}

fn warning_events(captured: &Captured) -> Result<Vec<Json>, Box<HelperError>> {
    captured
        .entries(Level::WARN)
        .iter()
        .map(|entry| as_json(json!({"subject": entry.subject, "status": entry.status})))
        .collect()
}

fn environment_of(input: &Environment) -> CountTokensEnvironment {
    CountTokensEnvironment {
        region_name: input.region_name.clone(),
        region: input.region.clone(),
        session_region: input.session_region.clone(),
        runtime_endpoint: input.runtime_endpoint.clone(),
        bearer_token: input.bearer_token.clone().map(SecretString::from),
    }
}

fn epoch(seconds: i64) -> Result<SystemTime, Box<HelperError>> {
    let bounded = u64::try_from(seconds).or_value_error()?;
    UNIX_EPOCH
        .checked_add(Duration::from_secs(bounded))
        .ok_or("now is out of range")
        .or_value_error()
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    let mut builder = Builder::new_current_thread();
    builder.enable_all();
    builder.build().or_value_error()
}

#[derive(Clone, Debug)]
struct Sent {
    method: Method,
    url: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    deadline: Duration,
}

#[derive(Clone)]
struct Script {
    reply: Arc<Mutex<Option<Reply>>>,
    sent: Arc<Mutex<Vec<Sent>>>,
}

impl Script {
    fn new(reply: &Reply) -> Self {
        Self {
            reply: Arc::new(Mutex::new(Some(reply.clone()))),
            sent: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn observed(&self) -> Result<Option<Json>, Box<HelperError>> {
        let sent = self
            .sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .first()
            .cloned();
        let Some(sent) = sent else {
            return Ok(None);
        };
        as_json(json!({
            "method": sent.method.as_str(),
            "url": sent.url,
            "body": String::from_utf8(sent.body).or_value_error()?,
            "timeout": sent.deadline.as_secs_f64(),
            "headers": reported_headers(&sent.headers),
        }))
        .map(Some)
    }

    fn answer(&self) -> Result<(u16, Vec<(String, String)>, Vec<u8>), TransportError> {
        let reply = self
            .reply
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let Some(reply) = reply else {
            return Err(TransportError::Closed);
        };
        if reply.failure {
            return Err(TransportError::Closed);
        }
        Ok((
            reply.status,
            reply.headers.into_iter().collect::<Vec<(String, String)>>(),
            reply.body.into_bytes(),
        ))
    }
}

impl CountTokensTransport for Script {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        record(&self.sent, request);
        let answer = self.answer();
        ready(answer.map(|(status, headers, body)| UpstreamResponse {
            status,
            headers: header_map(&headers),
            body,
        }))
    }
}

fn record(sent: &Mutex<Vec<Sent>>, request: OutboundRequest) {
    let OutboundRequest {
        method,
        url,
        headers,
        body,
        deadline,
        ..
    } = request;
    let payload = match body {
        RequestBody::Json(bytes) => bytes,
        RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
            Vec::new()
        }
    };
    let pairs = headers
        .iter()
        .map(|(name, value)| {
            (
                name.to_string(),
                value.to_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    sent.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(Sent {
            method,
            url: url.to_string(),
            headers: pairs,
            body: payload,
            deadline,
        });
}

fn reported_headers(headers: &[(String, String)]) -> BTreeMap<String, String> {
    headers
        .iter()
        .filter(|(name, _)| REPORTED_HEADERS.contains(&name.to_ascii_lowercase().as_str()))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.clone()))
        .collect()
}

fn header_map(pairs: &[(String, String)]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        let Ok(header_name) = HeaderName::from_bytes(name.as_bytes()) else {
            continue;
        };
        let Ok(header_value) = HeaderValue::from_str(value) else {
            continue;
        };
        headers.insert(header_name, header_value);
    }
    headers
}

struct FixtureCredentials {
    resolved: Arc<AtomicUsize>,
}

impl CountTokensCredentials for FixtureCredentials {
    fn resolve(
        &self,
        settings: AwsSettings,
        _region: String,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        self.resolved.fetch_add(1, Ordering::Relaxed);
        let credentials = match (settings.access_key_id, settings.secret_access_key) {
            (Some(access_key_id), Some(secret_access_key)) => Credentials {
                access_key_id,
                secret_access_key,
                session_token: settings.session_token,
            },
            _ => Credentials {
                access_key_id: SecretString::from("AKIDEXAMPLE".to_owned()),
                secret_access_key: SecretString::from("fixture-secret-key".to_owned()),
                session_token: None,
            },
        };
        ready(Ok::<Credentials, CredentialError>(credentials))
    }
}

struct EmptyHandler;

impl CountTokensHandler for EmptyHandler {
    fn handle_count_tokens_request(
        &self,
        _request: &CountTokensHandlerRequest<'_>,
        _now: SystemTime,
    ) -> impl Future<Output = Result<Option<CountResponse>, CountTokensHandlerError>> + Send {
        ready(Ok(None))
    }
}

struct EmptyObjectHandler;

impl CountTokensHandler for EmptyObjectHandler {
    fn handle_count_tokens_request(
        &self,
        _request: &CountTokensHandlerRequest<'_>,
        _now: SystemTime,
    ) -> impl Future<Output = Result<Option<CountResponse>, CountTokensHandlerError>> + Send {
        ready(Ok(Some(CountResponse::new())))
    }
}
