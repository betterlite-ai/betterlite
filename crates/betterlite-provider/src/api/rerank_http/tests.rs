#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_http_fixture::reply::reply as http_reply;
use betterlite_http_fixture::request::Request;
use betterlite_http_fixture::server::serve;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::RerankResponse;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::header::{HeaderMap as HttpHeaders, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde::de::DeserializeOwned;
use serde_json::Number;
use thiserror::Error;

use super::client::{
    ClientKind, ClientSource, RerankHttpRequest, RerankSendError, RerankTransport, SuppliedClient,
};
use super::error::{ClientError, RerankHttpError};
use super::{RerankCall, RerankDeps, RerankLogContext, rerank};
use crate::api::rerank_contracts::{
    BaseLlmException, BaseRerankConfig, EnvironmentRequest, HeaderMap, Headers,
    MapCohereRerankParams, Params, RerankLogging, RerankProviderResponse, ResponseTransform,
};

const INIT: &str = r#"{"model":"rerank-test","call_type":"rerank","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const REPLY: &str = r#"{"id":"r","results":[{"index":0,"relevance_score":0.5}]}"#;

fn parses<T: DeserializeOwned>(text: &str) -> T {
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

impl Fixture {
    fn new() -> Self {
        let init: LoggingInit = parses(INIT);
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
        let state = LoggingState::new(
            init,
            CorrelationSetup {
                context: &mut CorrelationContext::default(),
                settings: CorrelationSettings::default(),
            },
            &redactor,
            || String::from("generated"),
            Duration::default,
        )
        .expect("the logging state builds");
        Self {
            redactor,
            sink: Sink,
            state,
            now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00")
                .expect("the clock parses"),
        }
    }

    async fn rerank<C, S>(
        &mut self,
        deps: RerankDeps<'_, C, S>,
        call: RerankCall<'_>,
        events: &Events,
    ) -> Result<RerankResponse, RerankHttpError<C::Error>>
    where
        C: BaseRerankConfig,
        S: ClientSource,
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

    // the body the pre-call callback sees, through the additional_args LiteLLM stores
    fn logged_body(&self) -> Option<&Json> {
        let Json::Object(details) = self.state.model_call_details.get("additional_args")? else {
            return None;
        };
        details.get("complete_input_dict")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refusal {
    None,
    Validate,
    Transform,
    Response,
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

#[derive(Debug, Clone, PartialEq)]
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
    raw: Option<RerankProviderResponse>,
    transform: Option<SeenArgs>,
}

struct RecordingConfig {
    base: String,
    events: Events,
    refusal: Refusal,
    recorded: Mutex<Recorded>,
}

impl RecordingConfig {
    fn new(base: &str, events: &Events) -> Self {
        Self {
            base: base.to_owned(),
            events: events.clone(),
            refusal: Refusal::None,
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
        self.recorded().headers_in = Some(request.headers.clone());
        if self.refusal == Refusal::Validate {
            return Err(ConfigError::Validate);
        }
        let mut headers = request.headers.clone();
        let key = request.api_key.map_or_else(
            || String::from("sk-none"),
            |key| key.expose_secret().to_owned(),
        );
        headers.insert("x-api-key".to_owned(), key);
        Ok(headers)
    }

    fn transform_rerank_request(
        &self,
        _model: &str,
        optional: &Params,
        _headers: &Headers,
        _litellm_params: Option<&Params>,
    ) -> Result<Params, ConfigError> {
        self.events.push("transform");
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
        {
            let mut recorded = self.recorded();
            recorded.raw = Some(request.raw_response.clone());
            recorded.transform = Some(seen);
        }
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
        message: &str,
        status_code: u16,
        headers: HeaderMap,
    ) -> BaseLlmException {
        self.recorded()
            .errors
            .push((message.to_owned(), status_code, headers.clone()));
        BaseLlmException {
            status_code,
            message: format!("rerank:{message}"),
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

#[derive(Clone)]
struct Wire {
    label: &'static str,
    reply: Reply,
    failure: Option<RerankSendError>,
    events: Events,
    sent: Arc<Mutex<Vec<Sent>>>,
}

impl Wire {
    fn replying(label: &'static str, reply: &Reply, events: &Events) -> Self {
        Self {
            label,
            reply: reply.clone(),
            failure: None,
            events: events.clone(),
            sent: Arc::default(),
        }
    }

    fn sent(&self) -> MutexGuard<'_, Vec<Sent>> {
        self.sent.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn fail(&mut self, error: RerankSendError) {
        self.failure = Some(error);
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
        let answer = match &self.failure {
            Some(RerankSendError::Timeout) => Err(RerankSendError::Timeout),
            Some(RerankSendError::Failed { message }) => Err(RerankSendError::Failed {
                message: message.clone(),
            }),
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
            fixture: Fixture::new(),
            optional: params(r#"{"query": "hi", "documents": ["a"]}"#),
            key: SecretString::from(String::from("sk-caller")),
            model_response: response(
                r#"{"id": "caller", "results": [{"index": 1, "relevance_score": 0.5}]}"#,
            ),
            supplied_kind: ClientKind::Sync,
            use_supplied: true,
        }
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
            litellm_params: None,
            headers: None,
            timeout,
            is_async,
            model_response: self.model_response.clone(),
            logging,
        };
        let deps = RerankDeps {
            config: &self.config,
            source: &self.source,
            supplied: self.use_supplied.then(|| SuppliedClient {
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
    let base = format!("http://{}/v1/rerank", serving.address);
    let events = Events::default();
    let config = RecordingConfig::new(&base, &events);
    let optional = params(r#"{"query": "é", "documents": ["a"]}"#);
    let key = SecretString::from(String::from("sk-caller"));
    let model_response =
        response(r#"{"id": "caller", "results": [{"index": 3, "relevance_score": 0.25}]}"#);
    let client = proxied();
    let source = ProxiedSource {
        client: client.clone(),
    };
    let mut logging = RerankLogging::default();
    let call = RerankCall {
        model: "rerank-test",
        optional_params: &optional,
        api_base: None,
        api_key: Some(&key),
        litellm_params: None,
        headers: None,
        timeout: None,
        is_async: Json::Bool(false),
        model_response: model_response.clone(),
        logging: &mut logging,
    };
    let mut fixture = Fixture::new();
    let deps = RerankDeps {
        config: &config,
        source: &source,
        supplied: Some(SuppliedClient {
            kind: ClientKind::Sync,
            transport: &client,
        }),
    };
    let answer = fixture
        .rerank(deps, call, &events)
        .await
        .expect("the rerank succeeds");
    let request = serving.finish().expect("the server saw the request");
    assert_eq!(request.line, "POST /v1/rerank HTTP/1.1");
    assert_eq!(
        request.body,
        "{\"documents\": [\"a\"], \"query\": \"\\u00e9\"}"
    );
    assert_eq!(wire_headers(&request), only_api_key());
    assert_eq!(answer, model_response);
    assert_payload_transform(&config, &events);
    assert_payload_callback(&fixture, &logging);
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
        params(r#"{"query": "é", "documents": ["a"]}"#)
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

fn assert_payload_callback(fixture: &Fixture, logging: &RerankLogging) {
    let expected = params(r#"{"query": "é", "documents": ["a"]}"#);
    assert_eq!(fixture.input(), Some(&Json::Text(String::from("é"))));
    assert_eq!(fixture.logged_body(), Some(&object(&expected)));
    assert_eq!(
        logging.model_call_details.get("transformer_saw"),
        Some(&Json::Bool(true))
    );
}

#[tokio::test]
async fn rerank_http_execution_order() {
    execution_order(Json::Bool(false)).await;
    execution_order(Json::Bool(true)).await;
    integer_one_stays_sync().await;
    client_selection().await;
    timeout_forwarding().await;
    missing_query_logs_empty().await;
    failures_before_the_send().await;
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

async fn failures_before_the_send() {
    validate_failure().await;
    transform_failure().await;
    pre_call_failure().await;
}

async fn validate_failure() {
    let mut bench = Bench::new();
    bench.config.refusal = Refusal::Validate;
    let mut logging = RerankLogging::default();
    let outcome = bench.run(Json::Bool(false), None, &mut logging).await;
    assert!(matches!(
        outcome,
        Err(RerankHttpError::Config(ConfigError::Validate))
    ));
    assert_eq!(supplied_sent(&bench).len(), 0);
    assert!(bench.config.errors().is_empty());
}

async fn transform_failure() {
    let mut bench = Bench::new();
    bench.config.refusal = Refusal::Transform;
    let mut logging = RerankLogging::default();
    let outcome = bench.run(Json::Bool(false), None, &mut logging).await;
    assert!(matches!(
        outcome,
        Err(RerankHttpError::Config(ConfigError::Transform))
    ));
    assert_eq!(supplied_sent(&bench).len(), 0);
    assert!(bench.config.errors().is_empty());
}

async fn pre_call_failure() {
    let mut bench = Bench::new();
    bench.fixture.state.request_logging.raw_request_only = true;
    let mut logging = RerankLogging::default();
    let outcome = bench.run(Json::Bool(false), None, &mut logging).await;
    assert!(matches!(
        outcome,
        Err(RerankHttpError::Logging(PrePostError::RawRequestCaptured))
    ));
    assert_eq!(supplied_sent(&bench).len(), 0);
    assert!(bench.config.errors().is_empty());
}

#[tokio::test]
async fn rerank_http_provider_failure() {
    let mut bench = Bench::new();
    bench.supplied.reply = Reply::json(429, "slow").with_header("retry-after", "2");
    let mut logging = RerankLogging::default();
    let outcome = bench.run(Json::Bool(false), None, &mut logging).await;
    let Err(RerankHttpError::Provider(error)) = outcome else {
        panic!("the 429 becomes the provider error the config builds");
    };
    assert_eq!(error.status_code, 429);
    assert_eq!(error.message, "rerank:slow");
    let expected = HeaderMap::from([
        ("content-length".to_owned(), Json::Text(String::from("4"))),
        ("retry-after".to_owned(), Json::Text(String::from("2"))),
    ]);
    assert_eq!(error.headers, expected);
    assert_eq!(
        bench.config.errors(),
        vec![(String::from("slow"), 429, expected)]
    );
    assert_eq!(supplied_sent(&bench).len(), 1);
    assert_eq!(
        bench.events.recorded(),
        vec!["validate", "url", "transform", "pre_call", "post"]
    );
    response_transform_outside_conversion().await;
    send_failure_conversion().await;
}

async fn response_transform_outside_conversion() {
    let mut bench = Bench::new();
    bench.config.refusal = Refusal::Response;
    let mut logging = RerankLogging::default();
    let outcome = bench.run(Json::Bool(false), None, &mut logging).await;
    assert!(matches!(
        outcome,
        Err(RerankHttpError::Config(ConfigError::Response))
    ));
    assert_eq!(supplied_sent(&bench).len(), 1);
    assert!(bench.config.errors().is_empty());
}

async fn send_failure_conversion() {
    let mut bench = Bench::new();
    bench.supplied.fail(RerankSendError::Failed {
        message: String::from("connection refused"),
    });
    let mut logging = RerankLogging::default();
    let outcome = bench.run(Json::Bool(false), None, &mut logging).await;
    let Err(RerankHttpError::Provider(error)) = outcome else {
        panic!("a failed send becomes the provider error");
    };
    assert_eq!(error.status_code, 500);
    assert_eq!(error.message, "rerank:connection refused");
    assert_eq!(
        bench.config.errors(),
        vec![(String::from("connection refused"), 500, HeaderMap::new())]
    );
    assert!(
        !bench
            .events
            .recorded()
            .contains(&String::from("response_transform"))
    );
}

async fn timeout_conversion(is_async: Json, expected: &str) {
    let mut bench = Bench::new();
    bench.supplied_kind = if is_async == Json::Bool(true) {
        ClientKind::Asynchronous
    } else {
        ClientKind::Sync
    };
    bench.supplied.fail(RerankSendError::Timeout);
    let mut logging = RerankLogging::default();
    let outcome = bench
        .run(is_async, Some(Duration::from_secs(5)), &mut logging)
        .await;
    let Err(RerankHttpError::Provider(error)) = outcome else {
        panic!("a timeout becomes the provider error");
    };
    assert_eq!(error.status_code, 408);
    assert!(error.message.starts_with(expected));
}

#[tokio::test]
async fn rerank_http_timeout_messages() {
    timeout_conversion(
        Json::Bool(false),
        "rerank:litellm.Timeout: Connection timed out after 5 seconds.",
    )
    .await;
    timeout_conversion(
        Json::Bool(true),
        "rerank:litellm.Timeout: Connection timed out. Timeout passed=5, time taken=",
    )
    .await;
}

#[test]
fn requested_timeout_renders_python_floats() {
    assert_eq!(super::convert::requested_timeout(None), "None");
    assert_eq!(
        super::convert::requested_timeout(Some(Duration::from_secs(5))),
        "5"
    );
    assert_eq!(
        super::convert::requested_timeout(Some(Duration::from_millis(1500))),
        "1.5"
    );
}

#[test]
fn elapsed_text_trims_the_fraction() {
    assert_eq!(super::convert::elapsed_text(Duration::ZERO), "0.0");
    assert_eq!(
        super::convert::elapsed_text(Duration::from_millis(100)),
        "0.1"
    );
    assert_eq!(
        super::convert::elapsed_text(Duration::from_millis(12_345)),
        "12.345"
    );
    assert_eq!(
        super::convert::elapsed_text(Duration::from_millis(1_200)),
        "1.2"
    );
}

#[test]
fn client_kind_needs_the_boolean_true() {
    assert_eq!(
        super::client::client_kind(&Json::Bool(true)),
        ClientKind::Asynchronous
    );
    assert_eq!(
        super::client::client_kind(&Json::Bool(false)),
        ClientKind::Sync
    );
    assert_eq!(
        super::client::client_kind(&Json::Number(Number::from(1_u8))),
        ClientKind::Sync
    );
    assert_eq!(super::client::client_kind(&Json::Null), ClientKind::Sync);
    assert_eq!(
        super::client::client_kind(&Json::Text(String::from("true"))),
        ClientKind::Sync
    );
}
