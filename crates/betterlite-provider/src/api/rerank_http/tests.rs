#![cfg(test)]

mod failures;
mod units;

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::file_http_content::service::ClientSource;
use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_http_fixture::reply::reply as http_reply;
use betterlite_http_fixture::request::Request;
use betterlite_http_fixture::server::{Serving, serve};
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::RerankResponse;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::header::{HeaderMap as HttpHeaders, HeaderValue};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::de::DeserializeOwned;
use serde_json::Number;
use thiserror::Error;
use url::Url;

use super::client::{
    ClientKind, RerankHttpRequest, RerankSendError, RerankTransport, SuppliedClient,
};
use super::error::RerankHttpError;
use super::{RerankCall, RerankDeps, RerankLogContext, rerank};
use crate::api::rerank_contracts::{
    BaseLlmException, BaseRerankConfig, EnvironmentRequest, HeaderMap, Headers,
    MapCohereRerankParams, Params, RerankLogging, RerankProviderResponse, ResponseTransform,
};

const INIT: &str = r#"{"model":"rerank-test","call_type":"rerank","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const REPLY: &str = r#"{"id":"r","results":[{"index":0,"relevance_score":0.5}]}"#;

fn parses<T>(text: &str) -> T
where
    T: DeserializeOwned,
{
    serde_json::from_str(text).expect("the fixture parses")
}

fn params(text: &str) -> Params {
    parses(text)
}

fn response(text: &str) -> RerankResponse {
    parses(text)
}

fn object(data: &Params) -> Json {
    Json::Object(
        data.iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
    )
}

#[derive(Clone, Default)]
struct Events(Arc<Mutex<Vec<String>>>);

impl Events {
    fn push(&self, name: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(name.to_owned());
    }

    fn recorded(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

struct Fixture {
    redactor: Redactor,
    sink: Sink,
    state: LoggingState,
    now: DateTime<FixedOffset>,
}

impl Default for Fixture {
    fn default() -> Self {
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
        let mut context = CorrelationContext::default();
        let setup = CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        };
        let state = LoggingState::new(
            parses(INIT),
            setup,
            &redactor,
            || String::from("generated"),
            Duration::default,
        )
        .expect("the logging state builds");
        let now =
            DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").expect("the clock parses");
        Self {
            redactor,
            sink: Sink,
            state,
            now,
        }
    }
}

impl Fixture {
    async fn rerank<C, S>(
        &mut self,
        deps: RerankDeps<'_, C, S>,
        call: RerankCall<'_>,
        events: &Events,
    ) -> Result<RerankResponse, RerankHttpError<C::Error>>
    where
        C: BaseRerankConfig,
        S: ClientSource<Client: RerankTransport>,
    {
        let logger = |_details: &Json| -> Result<(), PrePostError> {
            events.push("pre_call");
            Ok(())
        };
        let env = PrePostEnv {
            debug: DebugSettings::default(),
            globals: GlobalFlags::default(),
            decision: Decision {
                content: false,
                key_info: false,
            },
            redactor: &self.redactor,
            log: &self.sink,
            input_callbacks: &[],
            sentry: None,
            logger_fn: Some(&logger),
            now: self.now,
        };
        rerank(
            call,
            deps,
            &mut RerankLogContext {
                state: &mut self.state,
                env: &env,
            },
        )
        .await
    }

    fn input(&self) -> Option<&Json> {
        self.state.model_call_details.get("input")
    }

    fn additional_args(&self) -> Option<&IndexMap<String, Json>> {
        let Json::Object(details) = self.state.model_call_details.get("additional_args")? else {
            return None;
        };
        Some(details)
    }

    // the body the pre-call callback sees, through the additional_args LiteLLM stores
    fn logged_body(&self) -> Option<&Json> {
        self.additional_args()?.get("complete_input_dict")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refusal {
    None,
    Validate,
    Transform,
    Response,
    Url,
    HeaderValue,
    HeaderName,
    PreCall,
}

impl Refusal {
    // only the response transformer refuses after the send; every other refusal stops the call before one
    const fn sent(self) -> usize {
        match self {
            Self::Response => 1,
            Self::None
            | Self::Validate
            | Self::Transform
            | Self::Url
            | Self::HeaderValue
            | Self::HeaderName
            | Self::PreCall => 0,
        }
    }
}

#[derive(Debug, Error)]
enum ConfigError {
    #[error("the fixture refuses validation")]
    Validate,
    #[error("the fixture refuses the request transform")]
    Transform,
    #[error("the fixture refuses the response transform")]
    Response,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SeenArgs {
    model: String,
    api_key: Option<String>,
    request_data: Params,
    optional_params: Params,
    litellm_params: Params,
}

#[derive(Default)]
struct Recorded {
    errors: Vec<(String, u16, HeaderMap)>,
    headers_in: Option<Headers>,
    validate_params: Option<Params>,
    params_in: Option<Params>,
    raw: Option<RerankProviderResponse>,
    transform: Option<SeenArgs>,
}

struct RecordingConfig {
    base: String,
    events: Events,
    refusal: Refusal,
    bad_value: bool,
    bad_name: bool,
    recorded: Mutex<Recorded>,
}

impl RecordingConfig {
    fn new(base: &str, events: &Events) -> Self {
        Self {
            base: base.to_owned(),
            events: events.clone(),
            refusal: Refusal::None,
            bad_value: false,
            bad_name: false,
            recorded: Mutex::default(),
        }
    }

    fn recorded(&self) -> MutexGuard<'_, Recorded> {
        self.recorded.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn errors(&self) -> Vec<(String, u16, HeaderMap)> {
        self.recorded().errors.clone()
    }
}

impl BaseRerankConfig for RecordingConfig {
    type Error = ConfigError;

    fn validate_environment(
        &self,
        request: EnvironmentRequest<'_>,
    ) -> Result<Headers, ConfigError> {
        self.events.push("validate");
        let mut recorded = self.recorded();
        recorded.headers_in = Some(request.headers.clone());
        recorded.validate_params = request.litellm_params.cloned();
        drop(recorded);
        if self.refusal == Refusal::Validate {
            return Err(ConfigError::Validate);
        }
        let mut headers = request.headers.clone();
        let key = request.api_key.map_or_else(
            || String::from("sk-none"),
            |key| key.expose_secret().to_owned(),
        );
        headers.insert("x-api-key".to_owned(), key);
        if self.bad_name {
            headers.insert("x bad".to_owned(), String::from("value"));
        }
        if self.bad_value {
            headers.insert("x-bad".to_owned(), String::from("bad\nvalue"));
        }
        Ok(headers)
    }

    fn transform_rerank_request(
        &self,
        _model: &str,
        optional: &Params,
        _headers: &Headers,
        litellm_params: Option<&Params>,
    ) -> Result<Params, ConfigError> {
        self.events.push("transform");
        self.recorded().params_in = litellm_params.cloned();
        if self.refusal == Refusal::Transform {
            return Err(ConfigError::Transform);
        }
        Ok(Params::from([
            (
                "query".to_owned(),
                optional
                    .get("query")
                    .cloned()
                    .unwrap_or_else(|| Json::Text(String::new())),
            ),
            (
                "documents".to_owned(),
                optional
                    .get("documents")
                    .cloned()
                    .unwrap_or(Json::List(Vec::new())),
            ),
        ]))
    }

    fn transform_rerank_response(
        &self,
        model_response: RerankResponse,
        request: ResponseTransform<'_>,
    ) -> Result<RerankResponse, ConfigError> {
        self.events.push("response_transform");
        request
            .logging
            .model_call_details
            .insert("transformer_saw".to_owned(), Json::Bool(true));
        let seen = SeenArgs {
            model: request.model.to_owned(),
            api_key: request.api_key.map(|key| key.expose_secret().to_owned()),
            request_data: request.request_data.clone(),
            optional_params: request.optional_params.clone(),
            litellm_params: request.litellm_params.clone(),
        };
        let mut recorded = self.recorded();
        recorded.raw = Some(request.raw_response.clone());
        recorded.transform = Some(seen);
        drop(recorded);
        if self.refusal == Refusal::Response {
            return Err(ConfigError::Response);
        }
        Ok(model_response)
    }

    fn get_complete_url(
        &self,
        _api_base: Option<&str>,
        _model: &str,
        _optional_params: Option<&Params>,
    ) -> String {
        self.events.push("url");
        self.base.clone()
    }

    fn supported_rerank_params(&self, _model: &str) -> Vec<String> {
        Vec::new()
    }

    fn map_cohere_rerank_params(
        &self,
        _request: MapCohereRerankParams<'_>,
    ) -> Result<Params, ConfigError> {
        Err(ConfigError::Transform)
    }

    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: HeaderMap,
    ) -> BaseLlmException {
        self.recorded()
            .errors
            .push((error_message.to_owned(), status_code, headers.clone()));
        BaseLlmException {
            status_code,
            message: format!("rerank:{error_message}"),
            headers,
        }
    }
}

#[derive(Clone)]
struct Reply {
    status: u16,
    headers: HttpHeaders,
    body: Vec<u8>,
}

impl Reply {
    fn json(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: HttpHeaders::new(),
            body: body.as_bytes().to_vec(),
        }
    }

    fn with_header(mut self, name: &'static str, value: &str) -> Self {
        self.headers.insert(
            name,
            HeaderValue::from_str(value).expect("the reply header"),
        );
        self
    }
}

struct Sent {
    label: &'static str,
    request: RerankHttpRequest,
    logging: bool,
}

// The mock transport fails the way the pooled one does: a timeout, or a typed send failure
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fault {
    Timeout,
    Send,
}

#[derive(Clone)]
struct Wire {
    label: &'static str,
    reply: Reply,
    fault: Option<Fault>,
    events: Events,
    sent: Arc<Mutex<Vec<Sent>>>,
}

impl Wire {
    fn replying(label: &'static str, reply: &Reply, events: &Events) -> Self {
        Self {
            label,
            reply: reply.clone(),
            fault: None,
            events: events.clone(),
            sent: Arc::default(),
        }
    }

    fn sent(&self) -> MutexGuard<'_, Vec<Sent>> {
        self.sent.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn fail(&mut self, fault: Fault) {
        self.fault = Some(fault);
    }
}

impl RerankTransport for Wire {
    fn send(
        &self,
        request: RerankHttpRequest,
        logging: Option<&RerankLogging>,
    ) -> impl Future<Output = Result<UpstreamResponse, RerankSendError>> + Send {
        self.events.push("post");
        self.sent().push(Sent {
            label: self.label,
            request,
            logging: logging.is_some(),
        });
        let answer = match self.fault {
            Some(Fault::Timeout) => Err(RerankSendError::Timeout),
            Some(Fault::Send) => Err(RerankSendError::Failed(HttpError::TooLarge { limit: 1024 })),
            None => Ok(UpstreamResponse {
                status: self.reply.status,
                headers: self.reply.headers.clone(),
                body: self.reply.body.clone(),
            }),
        };
        ready(answer)
    }
}

struct Source {
    client: Wire,
    created: Arc<Mutex<Vec<&'static str>>>,
}

impl Source {
    fn new(client: Wire) -> Self {
        Self {
            client,
            created: Arc::default(),
        }
    }

    fn created(&self) -> Vec<&'static str> {
        self.created
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ClientSource for Source {
    type Client = Wire;

    fn client(&self) -> Result<Wire, ClientError> {
        self.created
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(self.client.label);
        Ok(self.client.clone())
    }
}

struct ProxiedSource {
    client: ProxiedClient,
}

impl ClientSource for ProxiedSource {
    type Client = ProxiedClient;

    fn client(&self) -> Result<ProxiedClient, ClientError> {
        Ok(self.client.clone())
    }
}

fn proxied() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
        .expect("the proxy policy resolves");
    ProxiedClient::outbound(&Arc::new(policy)).expect("the proxied client builds")
}

struct Bench {
    config: RecordingConfig,
    supplied: Wire,
    pooled: Wire,
    source: Source,
    events: Events,
    fixture: Fixture,
    optional: Params,
    headers: Option<Headers>,
    litellm_params: Option<Params>,
    key: SecretString,
    model_response: RerankResponse,
    supplied_kind: ClientKind,
    use_supplied: bool,
}

impl Bench {
    fn new() -> Self {
        let events = Events::default();
        let supplied = Wire::replying("supplied", &Reply::json(200, REPLY), &events);
        let pooled = Wire::replying("pooled", &Reply::json(200, REPLY), &events);
        Self {
            config: RecordingConfig::new("http://mock.test/v1/rerank", &events),
            source: Source::new(pooled.clone()),
            supplied,
            pooled,
            events,
            fixture: Fixture::default(),
            optional: params(r#"{"query": "hi", "documents": ["a"]}"#),
            headers: None,
            litellm_params: None,
            key: SecretString::from(String::from("sk-caller")),
            model_response: response(
                r#"{"id": "caller", "results": [{"index": 1, "relevance_score": 0.5}]}"#,
            ),
            supplied_kind: ClientKind::Sync,
            use_supplied: true,
        }
    }

    // each refusal injects its failure where the matching production hook would hit it
    fn refusing(refusal: Refusal) -> Self {
        let mut bench = Self::new();
        bench.config.refusal = refusal;
        match refusal {
            Refusal::Url => bench.config.base = String::from("not a url"),
            Refusal::HeaderValue => bench.config.bad_value = true,
            Refusal::HeaderName => bench.config.bad_name = true,
            Refusal::PreCall => bench.fixture.state.request_logging.raw_request_only = true,
            Refusal::None | Refusal::Validate | Refusal::Transform | Refusal::Response => {}
        }
        bench
    }

    async fn run(
        &mut self,
        is_async: Json,
        timeout: Option<Duration>,
        logging: &mut RerankLogging,
    ) -> Result<RerankResponse, RerankHttpError<ConfigError>> {
        let call = RerankCall {
            model: "rerank-test",
            optional_params: &self.optional,
            api_base: None,
            api_key: Some(&self.key),
            litellm_params: self.litellm_params.as_ref(),
            headers: self.headers.as_ref(),
            timeout,
            is_async,
            model_response: self.model_response.clone(),
            logging,
        };
        let deps = RerankDeps {
            config: &self.config,
            source: &self.source,
            supplied: self.use_supplied.then_some(SuppliedClient {
                kind: self.supplied_kind,
                transport: &self.supplied,
            }),
        };
        self.fixture.rerank(deps, call, &self.events).await
    }
}

fn supplied_sent(bench: &Bench) -> MutexGuard<'_, Vec<Sent>> {
    bench.supplied.sent()
}

struct Payload {
    serving: Serving,
    base: String,
    config: RecordingConfig,
    events: Events,
    optional: Params,
    key: SecretString,
    model_response: RerankResponse,
    fixture: Fixture,
    logging: RerankLogging,
    answer: Option<RerankResponse>,
}

impl Payload {
    fn new(serving: Serving, events: Events) -> Self {
        let base = format!("http://{}/v1/rerank", serving.address);
        let config = RecordingConfig::new(&base, &events);
        let optional = params(r#"{"query": "\u00e9", "documents": ["a"]}"#);
        let key = SecretString::from(String::from("sk-caller"));
        let model_response =
            response(r#"{"id": "caller", "results": [{"index": 3, "relevance_score": 0.25}]}"#);
        Self {
            serving,
            base,
            config,
            events,
            optional,
            key,
            model_response,
            fixture: Fixture::default(),
            logging: RerankLogging::default(),
            answer: None,
        }
    }

    async fn run(&mut self) {
        let source = ProxiedSource { client: proxied() };
        let call = RerankCall {
            model: "rerank-test",
            optional_params: &self.optional,
            api_base: None,
            api_key: Some(&self.key),
            litellm_params: None,
            headers: None,
            timeout: None,
            is_async: Json::Bool(false),
            model_response: self.model_response.clone(),
            logging: &mut self.logging,
        };
        let deps = RerankDeps {
            config: &self.config,
            source: &source,
            supplied: None,
        };
        let answer = self
            .fixture
            .rerank(deps, call, &self.events)
            .await
            .expect("the rerank succeeds");
        self.answer = Some(answer);
    }
}

#[tokio::test]
async fn rerank_http_payload() {
    let serving = serve(
        http_reply(
            "200 OK",
            &[
                ("content-type", "application/json"),
                ("x-request-id", "req-1"),
            ],
            REPLY,
        ),
        Duration::ZERO,
    )
    .expect("the fixture server starts");
    let mut payload = Payload::new(serving, Events::default());
    payload.run().await;
    let request = payload
        .serving
        .finish()
        .expect("the server saw the request");
    assert_eq!(request.line, "POST /v1/rerank HTTP/1.1");
    assert_eq!(
        request.body,
        "{\"documents\": [\"a\"], \"query\": \"\\u00e9\"}"
    );
    assert_eq!(wire_headers(&request), only_api_key());
    assert_eq!(payload.answer.as_ref(), Some(&payload.model_response));
    assert_payload_transform(&payload.config, &payload.events);
    assert_payload_callback(&payload.fixture, &payload.logging, &payload.base);
}

fn wire_headers(request: &Request) -> BTreeMap<String, String> {
    request
        .head
        .lines()
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .filter(|(name, _)| {
            !matches!(
                name.as_str(),
                "host"
                    | "content-length"
                    | "connection"
                    | "accept"
                    | "accept-encoding"
                    | "user-agent"
                    | "transfer-encoding"
            )
        })
        .collect()
}

fn only_api_key() -> BTreeMap<String, String> {
    BTreeMap::from([("x-api-key".to_owned(), "sk-caller".to_owned())])
}

fn assert_payload_transform(config: &RecordingConfig, events: &Events) {
    let recorded = config.recorded();
    assert_eq!(recorded.headers_in.as_ref(), Some(&Headers::new()));
    assert_eq!(recorded.validate_params, None);
    assert_eq!(recorded.params_in, None);
    let raw = recorded.raw.as_ref().expect("the transformer saw a reply");
    assert_eq!(raw.status_code, 200);
    assert_eq!(raw.body, REPLY.as_bytes());
    assert_eq!(
        raw.headers.get("x-request-id").map(String::as_str),
        Some("req-1")
    );
    let seen = recorded.transform.as_ref().expect("the transformer ran");
    assert_eq!(seen.model, "rerank-test");
    assert_eq!(seen.api_key.as_deref(), Some("sk-caller"));
    assert_eq!(
        seen.request_data,
        params(r#"{"query": "\u00e9", "documents": ["a"]}"#)
    );
    assert_eq!(seen.optional_params, Params::new());
    assert_eq!(seen.litellm_params, Params::new());
    drop(recorded);
    assert_eq!(
        events.recorded(),
        vec![
            "validate",
            "url",
            "transform",
            "pre_call",
            "response_transform"
        ]
    );
}

fn assert_payload_callback(fixture: &Fixture, logging: &RerankLogging, api_base: &str) {
    let expected = params(r#"{"query": "\u00e9", "documents": ["a"]}"#);
    assert_eq!(fixture.input(), Some(&Json::Text(String::from("\u{e9}"))));
    assert_eq!(fixture.logged_body(), Some(&object(&expected)));
    let args = fixture
        .additional_args()
        .expect("the pre-call stored its arguments");
    assert_eq!(args.get("api_base"), Some(&Json::Text(api_base.to_owned())));
    let masked = IndexMap::from([("x-api-key".to_owned(), Json::Text(String::from("sk****er")))]);
    assert_eq!(args.get("headers"), Some(&Json::Object(masked)));
    assert_eq!(
        logging.model_call_details.get("transformer_saw"),
        Some(&Json::Bool(true))
    );
    // LiteLLM stores the raw key for every callback; the shared pre-call stores the masked header instead
    assert!(fixture.state.model_call_details.get("api_key").is_none());
    let details = Json::Object(fixture.state.model_call_details.clone());
    let text = serde_json::to_string(&details).expect("the callback details serialize");
    assert!(!text.contains("sk-caller"));
}

// LiteLLM hands the caller's headers and litellm_params to the config, and a provider reads an auth header or parameter from them
#[tokio::test]
async fn rerank_http_forwards_caller_headers_and_params() {
    let mut bench = Bench::new();
    let headers = Headers::from([("authorization".to_owned(), String::from("Bearer caller"))]);
    let litellm_params = params(r#"{"azure_ai_api_version": "2024-01-01"}"#);
    bench.headers = Some(headers.clone());
    bench.litellm_params = Some(litellm_params.clone());
    let mut logging = RerankLogging::default();
    bench
        .run(Json::Bool(false), None, &mut logging)
        .await
        .expect("the rerank succeeds");
    let recorded = bench.config.recorded();
    assert_eq!(recorded.headers_in.as_ref(), Some(&headers));
    assert_eq!(recorded.validate_params.as_ref(), Some(&litellm_params));
    assert_eq!(recorded.params_in.as_ref(), Some(&litellm_params));
    drop(recorded);
    let authorization = supplied_sent(&bench)[0]
        .request
        .headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    assert_eq!(authorization.as_deref(), Some("Bearer caller"));
}

// httpx follows a redirect with the caller's headers attached; the pooled client returns the 3xx as the provider's answer
#[tokio::test]
async fn rerank_http_never_follows_a_redirect() {
    let serving = serve(
        http_reply("302 Found", &[("location", "http://127.0.0.1:1/moved")], ""),
        Duration::ZERO,
    )
    .expect("the fixture server starts");
    let client = proxied();
    let request = RerankHttpRequest {
        url: Url::parse(&format!("http://{}/v1/rerank", serving.address)).expect("the mock URL"),
        headers: HttpHeaders::new(),
        body: b"{}".to_vec(),
        timeout: None,
    };
    let answer = RerankTransport::send(&client, request, None)
        .await
        .expect("the redirect is the answer itself");
    assert_eq!(answer.status, 302);
    let seen = serving.finish().expect("the server saw the request");
    assert_eq!(seen.line, "POST /v1/rerank HTTP/1.1");
}

#[tokio::test]
async fn rerank_http_execution_order() {
    execution_order(Json::Bool(false)).await;
    execution_order(Json::Bool(true)).await;
    integer_one_stays_sync().await;
    client_selection().await;
    timeout_forwarding().await;
    missing_query_logs_empty().await;
    failures::before_the_send().await;
}

async fn execution_order(is_async: Json) {
    let mut bench = Bench::new();
    let mut logging = RerankLogging::default();
    let asynchronous = is_async == Json::Bool(true);
    bench.supplied_kind = if asynchronous {
        ClientKind::Asynchronous
    } else {
        ClientKind::Sync
    };
    let answer = bench
        .run(is_async, None, &mut logging)
        .await
        .expect("the rerank succeeds");
    assert_eq!(
        bench.events.recorded(),
        vec![
            "validate",
            "url",
            "transform",
            "pre_call",
            "post",
            "response_transform"
        ]
    );
    let sent = supplied_sent(&bench);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].label, "supplied");
    assert_eq!(sent[0].request.timeout, None);
    assert_eq!(sent[0].logging, asynchronous);
    drop(sent);
    assert_eq!(answer, bench.model_response);
    assert!(bench.source.created().is_empty());
}

async fn integer_one_stays_sync() {
    let mut bench = Bench::new();
    let mut logging = RerankLogging::default();
    bench
        .run(Json::Number(Number::from(1_u8)), None, &mut logging)
        .await
        .expect("the rerank succeeds");
    assert!(!supplied_sent(&bench)[0].logging);
}

async fn client_selection() {
    let mut bench = Bench::new();
    let mut logging = RerankLogging::default();
    bench.supplied_kind = ClientKind::Asynchronous;
    bench
        .run(Json::Bool(false), None, &mut logging)
        .await
        .expect("the wrong-kind client is replaced");
    assert_eq!(supplied_sent(&bench).len(), 0);
    assert_eq!(bench.source.created(), vec!["pooled"]);
    assert_eq!(bench.pooled.sent().len(), 1);
}

async fn timeout_forwarding() {
    let mut bench = Bench::new();
    let mut logging = RerankLogging::default();
    bench
        .run(
            Json::Bool(false),
            Some(Duration::from_secs(5)),
            &mut logging,
        )
        .await
        .expect("the rerank succeeds");
    assert_eq!(
        supplied_sent(&bench)[0].request.timeout,
        Some(Duration::from_secs(5))
    );
}

async fn missing_query_logs_empty() {
    let mut bench = Bench::new();
    bench.optional = params(r#"{"documents": ["a"]}"#);
    let mut logging = RerankLogging::default();
    bench
        .run(Json::Bool(false), None, &mut logging)
        .await
        .expect("the rerank succeeds");
    assert_eq!(bench.fixture.input(), Some(&Json::Text(String::new())));
}
