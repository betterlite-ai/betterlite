use std::collections::VecDeque;
use std::fmt::{Debug, Formatter, Result as FmtResult};
use std::future::{Future, ready};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use http::Method;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use secrecy::SecretString;
use serde_json::value::{RawValue, to_raw_value};

use crate::aws_settings::sources::Credentials;
use crate::count_call::counter::{CountTokensCall, CountTokensDeployment};
use crate::count_call::handler::{
    BedrockCountTokensHandler, CountTokensCredentials, CountTokensEnvironment, CountTokensHandler,
    CountTokensHandlerError, CountTokensHandlerRequest, CountTokensTransport, ParamMap,
};
use crate::count_call::reply::CountResponse;
use crate::credentials::error::CredentialError;

pub(crate) const MODEL: &str = "bedrock/converse/us.anthropic.claude-3-5-sonnet-20241022-v2:0";
pub(crate) const REQUEST_MODEL: &str = "requested-model";

#[derive(Debug, Clone)]
pub(crate) struct Sent {
    pub(crate) method: Method,
    pub(crate) url: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
    pub(crate) deadline: Duration,
}

type Answer = (u16, Vec<(String, String)>, Vec<u8>);

#[derive(Debug, Clone)]
pub(crate) enum Reply {
    Answer {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
    Failure,
}

impl Reply {
    pub(crate) fn ok(body: &str) -> Self {
        Self::status(200, body)
    }

    pub(crate) fn status(status: u16, body: &str) -> Self {
        Self::Answer {
            status,
            headers: Vec::new(),
            body: body.as_bytes().to_vec(),
        }
    }

    pub(crate) fn with_headers(status: u16, headers: &[(&str, &str)], body: &str) -> Self {
        Self::Answer {
            status,
            headers: headers
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
            body: body.as_bytes().to_vec(),
        }
    }
}

#[derive(Clone)]
pub(crate) struct Script {
    replies: Arc<Mutex<VecDeque<Reply>>>,
    sent: Arc<Mutex<Vec<Sent>>>,
}

impl Debug for Script {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("Script").finish_non_exhaustive()
    }
}

impl Script {
    pub(crate) fn replying(reply: Reply) -> Self {
        Self {
            replies: Arc::new(Mutex::new(VecDeque::from([reply]))),
            sent: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) fn requests(&self) -> Vec<Sent> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn request(&self) -> Sent {
        let mut requests = self.requests();
        assert_eq!(requests.len(), 1);
        requests.remove(0)
    }

    fn reply(&self) -> Result<Answer, TransportError> {
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match reply {
            None | Some(Reply::Failure) => Err(TransportError::Closed),
            Some(Reply::Answer {
                status,
                headers,
                body,
            }) => Ok((status, headers, body)),
        }
    }

    fn record(&self, request: OutboundRequest) {
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
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Sent {
                method,
                url: url.to_string(),
                headers: pairs,
                body: payload,
                deadline,
            });
    }
}

impl CountTokensTransport for Script {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.record(request);
        let reply = self.reply();
        ready(reply.map(|(status, headers, body)| UpstreamResponse {
            status,
            headers: header_map(&headers),
            body,
        }))
    }
}

pub(crate) fn header_map(pairs: &[(String, String)]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        let header_name = HeaderName::from_bytes(name.as_bytes()).unwrap();
        headers.insert(header_name, HeaderValue::from_str(value).unwrap());
    }
    headers
}

#[derive(Debug, Clone)]
pub(crate) struct FixtureCredentials {
    pub(crate) credentials: Credentials,
    pub(crate) calls: Arc<AtomicUsize>,
}

impl FixtureCredentials {
    pub(crate) fn new(
        access_key_id: &str,
        secret_access_key: &str,
        session_token: Option<&str>,
    ) -> Self {
        Self {
            credentials: Credentials {
                access_key_id: SecretString::from(access_key_id.to_owned()),
                secret_access_key: SecretString::from(secret_access_key.to_owned()),
                session_token: session_token.map(|token| SecretString::from(token.to_owned())),
            },
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub(crate) fn calls(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }
}

impl CountTokensCredentials for FixtureCredentials {
    fn resolve(
        &self,
        settings: AwsSettings,
        _region: String,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        self.calls.fetch_add(1, Ordering::Relaxed);
        // B594's static-key branch hands the deployment's own keys straight back
        let credentials = match (settings.access_key_id, settings.secret_access_key) {
            (Some(access_key_id), Some(secret_access_key)) => Credentials {
                access_key_id,
                secret_access_key,
                session_token: settings.session_token,
            },
            _ => self.credentials.clone(),
        };
        ready(Ok::<Credentials, CredentialError>(credentials))
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FailingCredentials {
    pub(crate) calls: Arc<AtomicUsize>,
}

impl FailingCredentials {
    pub(crate) fn new() -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl CountTokensCredentials for FailingCredentials {
    fn resolve(
        &self,
        _settings: AwsSettings,
        _region: String,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        self.calls.fetch_add(1, Ordering::Relaxed);
        ready(Err(CredentialError::MissingCredentials))
    }
}

pub(crate) struct FixedHandler {
    pub(crate) result: Option<CountResponse>,
}

impl CountTokensHandler for FixedHandler {
    fn handle_count_tokens_request(
        &self,
        _request: &CountTokensHandlerRequest<'_>,
        _now: SystemTime,
    ) -> impl Future<Output = Result<Option<CountResponse>, CountTokensHandlerError>> + Send {
        ready(Ok(self.result.clone()))
    }
}

pub(crate) type Handler = BedrockCountTokensHandler<Script, FixtureCredentials>;

pub(crate) struct Fixture {
    pub(crate) handler: Handler,
    pub(crate) transport: Script,
    pub(crate) credentials: FixtureCredentials,
}

pub(crate) fn fixture(reply: Reply, environment: CountTokensEnvironment) -> Fixture {
    let transport = Script::replying(reply);
    let credentials = FixtureCredentials::new("AKIDEXAMPLE", "fixture-secret-key", None);
    Fixture {
        handler: BedrockCountTokensHandler {
            transport: transport.clone(),
            credentials: credentials.clone(),
            environment,
        },
        transport,
        credentials,
    }
}

pub(crate) fn environment(entries: &[(&str, &str)]) -> CountTokensEnvironment {
    let mut environment = CountTokensEnvironment::default();
    for (name, value) in entries {
        match *name {
            "AWS_REGION_NAME" => environment.region_name = Some((*value).to_owned()),
            "AWS_REGION" => environment.region = Some((*value).to_owned()),
            "AWS_BEDROCK_RUNTIME_ENDPOINT" => {
                environment.runtime_endpoint = Some((*value).to_owned());
            }
            "AWS_BEARER_TOKEN_BEDROCK" => {
                environment.bearer_token = Some(SecretString::from((*value).to_owned()));
            }
            other => panic!("unknown environment variable {other}"),
        }
    }
    environment
}

pub(crate) fn deployment(entries: &[(&str, Json)]) -> CountTokensDeployment {
    let mut litellm_params = ParamMap::new();
    for (name, value) in entries {
        litellm_params.insert((*name).to_owned(), to_raw(value));
    }
    CountTokensDeployment { litellm_params }
}

pub(crate) fn to_raw(value: &Json) -> Box<RawValue> {
    to_raw_value(value).unwrap()
}

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub(crate) fn object(value: Json) -> CountResponse {
    match value {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("expected a JSON object")
        }
    }
}

pub(crate) fn user_messages() -> Json {
    json(r#"[{"role":"user","content":"hi"}]"#)
}

pub(crate) fn now() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_hours(490_896))
        .unwrap()
}

pub(crate) fn call<'call>(
    messages: &'call Json,
    deployment: Option<&'call CountTokensDeployment>,
) -> CountTokensCall<'call> {
    CountTokensCall {
        model_to_use: MODEL,
        messages: Some(messages),
        deployment,
        request_model: REQUEST_MODEL,
        tools: None,
        system: None,
        now: now(),
    }
}

pub(crate) fn header<'sent>(sent: &'sent Sent, name: &str) -> Option<&'sent str> {
    sent.headers
        .iter()
        .find(|(present, _)| present == name)
        .map(|(_, value)| value.as_str())
}
