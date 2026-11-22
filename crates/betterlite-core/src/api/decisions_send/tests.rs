use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::net::TcpListener;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, ThreadId};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_config::types::ProxyTrust;
use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_http::interactions::base::clients::HttpClients;
use betterlite_http::interactions::base::{ClientError, Params, RequestFailure};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http_fixture::reply::reply;
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
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::systemone;
use betterlite_models::decisions::{DecisionsJson, Extra, Json, TokenCount, ir, openai};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::decisions::call::azure::AzureAiConfig;
use betterlite_openai::decisions::call::fields::{
    RequestFields, RequestFormat, SystemOneRequestFields,
};
use betterlite_openai::decisions::call::provider::ProviderConfig;
use betterlite_openai::decisions::call::{DecisionsCall, PrepareCall, prepare_call};
use betterlite_openai::decisions::openai::request::wire::{
    InputWire, QuestionWire, RequestWire as OpenAiRequestWire,
};
use betterlite_openai::decisions::openai::response::ResponseConversionError;
use betterlite_openai::decisions::transformation::provider_config::{
    ParseResponseError, SigningContext,
};
use betterlite_openai::decisions::transformation::wire::{
    ProviderRequestWire, RequestWire as SystemOneRequestWire,
};
use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::providers::typesafe::decisions::TypeSafeDecisionsConfig;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use http::header::{HeaderMap, HeaderName, HeaderValue};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret, SecretString};
use serde_json::Number;
use url::Url;

use super::clients::{
    DecisionsAsyncClient, DecisionsBody, DecisionsClients, DecisionsRequest, DecisionsSyncClient,
};
use super::config::DecisionsSendConfig;
use super::error::{DecisionsSendError, provider_error, send_failure};
use super::format::{DecisionsOutput, format_response};
use super::send::DecisionsSender;
use crate::api::vector_store_http_write::error::BaseLlmException;
use crate::api::vector_store_http_write::service::ClientKind;
use crate::helpers::RESPONSE_COST_HEADER;

const SYSTEMONE_ANSWER: &str = r#"{"model":"model-1","answers":{"first":{"type":"noul","noul":0.9}},"usage":{"input_tokens":3,"output_tokens":5}}"#;

#[derive(Clone, Default)]
struct Events(Arc<Mutex<Vec<String>>>);

impl Events {
    fn new() -> Self {
        Self::default()
    }

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

fn expected(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

type CallLogger = Box<dyn Fn(&Json) -> Result<(), PrePostError> + Sync>;

// The logging environment a pre_call needs, with a logger fn that records what it was handed
struct LogEnv {
    redactor: Redactor,
    sink: Sink,
    now: chrono::DateTime<chrono::FixedOffset>,
    logger: CallLogger,
}

impl LogEnv {
    fn for_events(events: &Events) -> Self {
        let recorded = events.clone();
        let logger = Box::new(move |_details: &Json| -> Result<(), PrePostError> {
            recorded.push("log");
            Ok(())
        });
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
        let now = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00")
            .expect("the clock parses");
        Self {
            redactor,
            sink: Sink,
            now,
            logger,
        }
    }

    fn env(&self) -> PrePostEnv<'_> {
        PrePostEnv {
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
            logger_fn: Some(self.logger.as_ref()),
            now: self.now,
        }
    }
}

fn logging_state() -> LoggingState {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"decision","call_type":"decisions","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1"}"#,
    )
    .expect("the logging fixture is valid");
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("generated"),
        Duration::default,
    )
    .expect("the logging state builds")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Validated {
    headers: Vec<(String, String)>,
    model: String,
    api_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SignSeen {
    headers: Vec<(String, String)>,
    url: String,
    api_base: String,
    api_key: Option<String>,
    params_empty: bool,
}

#[derive(Default)]
struct Recording {
    events: Events,
    validations: Mutex<Vec<Validated>>,
    urls: Mutex<Vec<(String, String)>>,
    signs: Mutex<Vec<SignSeen>>,
    errors: Mutex<Vec<(String, u16, Headers)>>,
    threads: Mutex<Vec<ThreadId>>,
}

impl Recording {
    fn seen_validations(&self) -> Vec<Validated> {
        self.validations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn seen_urls(&self) -> Vec<(String, String)> {
        self.urls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn seen_signs(&self) -> Vec<SignSeen> {
        self.signs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn seen_errors(&self) -> Vec<(String, u16, Headers)> {
        self.errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn sign_threads(&self) -> Vec<ThreadId> {
        self.threads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

enum Transform {
    Answer(ir::DecisionResponse),
    Invalid(String),
    Malformed,
}

// The fake config answers every provider call the send stage makes and records it
struct TestConfig {
    recording: Arc<Recording>,
    transform: Mutex<Transform>,
    cost: Option<Decimal>,
    aws: bool,
    signature: Option<String>,
    signed_body: Option<Vec<u8>>,
}

impl TestConfig {
    fn new(recording: &Arc<Recording>) -> Self {
        Self {
            recording: recording.clone(),
            transform: Mutex::new(Transform::Answer(ir_response(Some("model-1")))),
            cost: None,
            aws: false,
            signature: None,
            signed_body: None,
        }
    }
}

impl DecisionsSendConfig for TestConfig {
    fn validate_environment(
        &self,
        headers: &Headers,
        model: &str,
        api_key: Option<&SecretString>,
    ) -> Headers {
        self.recording.events.push("validate");
        self.recording
            .validations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Validated {
                headers: pairs(headers),
                model: model.to_owned(),
                api_key: api_key.map(|key| key.expose_secret().to_owned()),
            });
        let mut outbound = headers.clone();
        if let Some(key) = api_key {
            outbound.insert(
                "Authorization".to_owned(),
                format!("Bearer {}", key.expose_secret()),
            );
        }
        outbound.insert("Content-Type".to_owned(), String::from("application/json"));
        outbound
    }

    fn get_complete_url(&self, api_base: &str, model: &str) -> String {
        self.recording.events.push("url");
        self.recording
            .urls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((api_base.to_owned(), model.to_owned()));
        format!("{}/v1/systemone", api_base.trim_end_matches('/'))
    }

    fn signs_with_aws(&self, _api_base: &str) -> bool {
        self.aws
    }

    fn sign_request(
        &self,
        mut headers: Headers,
        context: SigningContext<'_>,
    ) -> (Headers, Option<Vec<u8>>) {
        self.recording.events.push("sign");
        self.recording
            .threads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(thread::current().id());
        self.recording
            .signs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(SignSeen {
                headers: pairs(&headers),
                url: context.url.to_owned(),
                api_base: context.api_base.to_owned(),
                api_key: context.api_key.map(|key| key.expose_secret().to_owned()),
                params_empty: context.litellm_params.is_empty(),
            });
        if let Some(signature) = &self.signature {
            headers.insert("X-Signature".to_owned(), signature.clone());
        }
        (headers, self.signed_body.clone())
    }

    fn transform_decisions_response(
        &self,
        _model: &str,
        _custom_llm_provider: &str,
        raw_response: &[u8],
        _request: &ir::DecisionRequest,
    ) -> Result<ir::DecisionResponse, ParseResponseError> {
        self.recording.events.push("transform");
        let _ = raw_response;
        match &*self
            .transform
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
        {
            Transform::Answer(response) => Ok(response.clone()),
            Transform::Invalid(message) => Err(ParseResponseError::Invalid(Box::new(
                provider_error_of(message),
            ))),
            Transform::Malformed => {
                let error =
                    serde_json::from_str::<Json>("{not json").expect_err("the fixture is not JSON");
                Err(ParseResponseError::Malformed(error))
            }
        }
    }

    fn provider_reported_cost(&self, _response: &ir::DecisionResponse) -> Option<Decimal> {
        self.cost
    }

    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: &Headers,
    ) -> ProviderError {
        self.recording
            .errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((error_message.to_owned(), status_code, headers.clone()));
        provider_error_of(error_message, status_code)
    }
}

fn provider_error_of(message: &str, status_code: u16) -> ProviderError {
    ProviderError::from(ProviderErrorParts {
        status_code,
        message: message.to_owned(),
        headers: None,
        request: None,
        response: None,
        body: None,
        status_code_is_synthesized: false,
    })
}

struct FakeAnswer {
    status: u16,
    headers: HeaderMap,
    body: Vec<u8>,
}

// The wire a decisions call sends through: it records the request and answers or fails with the fixture's choice
#[derive(Clone)]
struct FakeWire {
    recording: Arc<Recording>,
    answer: Arc<Mutex<FakeAnswer>>,
    failure: Arc<Mutex<Option<RequestFailure>>>,
    requests: Arc<Mutex<Vec<DecisionsRequest>>>,
}

impl FakeWire {
    fn new(recording: &Arc<Recording>, status: u16, body: &str) -> Self {
        Self {
            recording: recording.clone(),
            answer: Arc::new(Mutex::new(FakeAnswer {
                status,
                headers: HeaderMap::new(),
                body: body.as_bytes().to_vec(),
            })),
            failure: Arc::default(),
            requests: Arc::default(),
        }
    }

    fn failing(recording: &Arc<Recording>, failure: RequestFailure) -> Self {
        let wire = Self::new(recording, 200, "{}");
        *wire.failure.lock().unwrap_or_else(PoisonError::into_inner) = Some(failure);
        wire
    }

    fn answering(&self, status: u16, body: &[u8]) {
        let mut answer = self.answer.lock().unwrap_or_else(PoisonError::into_inner);
        answer.status = status;
        answer.body = body.to_vec();
    }

    fn with_header(&self, name: &'static str, value: &'static str) {
        self.answer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .headers
            .append(
                HeaderName::from_static(name),
                HeaderValue::from_static(value),
            );
    }

    fn requests(&self) -> MutexGuard<'_, Vec<DecisionsRequest>> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn record(&self, request: DecisionsRequest) -> Result<UpstreamResponse, RequestFailure> {
        self.recording.events.push("send");
        let failure = self
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        failure.map_or_else(
            || {
                let answer = self.answer.lock().unwrap_or_else(PoisonError::into_inner);
                Ok(UpstreamResponse {
                    status: answer.status,
                    headers: answer.headers.clone(),
                    body: answer.body.clone(),
                })
            },
            Err,
        )
    }
}

impl DecisionsSyncClient for FakeWire {
    fn send_blocking(&self, request: DecisionsRequest) -> Result<UpstreamResponse, RequestFailure> {
        self.record(request)
    }
}

impl DecisionsAsyncClient for FakeWire {
    fn send(
        &self,
        request: DecisionsRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, RequestFailure>> + Send {
        ready(self.record(request))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Sync,
    Asynchronous,
}

struct FakeClients {
    recording: Arc<Recording>,
    wire: FakeWire,
    failure: Option<ClientError>,
    selections: Mutex<Vec<(Mode, Params)>>,
}

impl FakeClients {
    fn from_wire(recording: &Arc<Recording>, wire: FakeWire) -> Self {
        Self {
            recording: recording.clone(),
            wire,
            failure: None,
            selections: Mutex::default(),
        }
    }

    fn failing(recording: &Arc<Recording>, wire: FakeWire, failure: ClientError) -> Self {
        let mut clients = Self::from_wire(recording, wire);
        clients.failure = Some(failure);
        clients
    }

    fn selections(&self) -> Vec<(Mode, Params)> {
        self.selections
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn selected(&mut self, mode: Mode, params: &Params) -> Result<FakeWire, ClientError> {
        self.recording.events.push(match mode {
            Mode::Sync => "select-sync",
            Mode::Asynchronous => "select-async",
        });
        self.selections
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((mode, params.clone()));
        self.failure
            .take()
            .map_or_else(|| Ok(self.wire.clone()), Err)
    }
}

impl DecisionsClients for FakeClients {
    type AsyncClient = FakeWire;
    type SyncClient = FakeWire;

    fn select_sync(&mut self, params: &Params) -> Result<FakeWire, ClientError> {
        self.selected(Mode::Sync, params)
    }

    fn select_async(&mut self, params: &Params) -> Result<FakeWire, ClientError> {
        self.selected(Mode::Asynchronous, params)
    }
}

// One test's config, logging environment, clients and wire, so a case only builds its call
struct Fixture {
    config: TestConfig,
    log: LogEnv,
    clients: FakeClients,
    wire: FakeWire,
    recording: Arc<Recording>,
}

impl Fixture {
    fn new() -> Self {
        let recording = Arc::new(Recording::default());
        let wire = FakeWire::new(&recording, 200, SYSTEMONE_ANSWER);
        let clients = FakeClients::from_wire(&recording, wire.clone());
        Self {
            config: TestConfig::new(&recording),
            log: LogEnv::for_events(&recording.events),
            clients,
            wire,
            recording,
        }
    }

    fn failing(failure: RequestFailure) -> Self {
        let mut fixture = Self::new();
        let wire = FakeWire::failing(&fixture.recording, failure);
        fixture.clients = FakeClients::from_wire(&fixture.recording, wire.clone());
        fixture.wire = wire;
        fixture
    }

    fn send(&mut self, call: &DecisionsCall<'_>) -> Result<DecisionsOutput, DecisionsSendError> {
        let env = self.log.env();
        let mut sender = DecisionsSender::new(&mut self.clients, &env);
        sender.send_with(call, &self.config)
    }

    async fn asend(
        &mut self,
        call: &DecisionsCall<'_>,
    ) -> Result<DecisionsOutput, DecisionsSendError> {
        let env = self.log.env();
        let mut sender = DecisionsSender::new(&mut self.clients, &env);
        sender.asend_with(call, &self.config).await
    }
}

fn pairs(headers: &Headers) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn answer_names(response: &systemone::DecisionsResponse) -> Vec<String> {
    response
        .answers
        .keys()
        .map(|name| name.as_ref().clone())
        .collect()
}

fn systemone_request() -> systemone::DecisionsRequest {
    serde_json::from_str(
        r#"{"model":"model-1","state":"state","questions":{"first":{"type":"noul","instructions":"Is it true?"}}}"#,
    )
    .expect("the request fixture parses")
}

fn openai_request() -> openai::DecisionRequest {
    serde_json::from_str(
        r#"{"model":"model-1","input":"state","questions":[{"type":"predicate","name":"first","instructions":"Is it true?"}]}"#,
    )
    .expect("the request fixture parses")
}

fn ir_request() -> ir::DecisionRequest {
    ir::DecisionRequest {
        input: ir::DecisionInput::State(DecisionsJson::Text(String::from("state"))),
        questions: vec![ir::DecisionQuestion::Predicate(ir::PredicateQuestion {
            name: Some(String::from("first")),
            instructions: Some(DecisionsJson::Text(String::from("Is it true?"))),
            criteria: None,
            extra: Extra::new(),
        })],
        safety_identifier: None,
    }
}

fn ir_response(model: Option<&str>) -> ir::DecisionResponse {
    ir::DecisionResponse {
        model: model.map(str::to_owned),
        answers: vec![ir::DecisionAnswer::Predicate {
            probability: Number::from_f64(0.9).expect("a finite probability"),
            extra: Extra::new(),
        }],
        usage: ir::DecisionUsage {
            input_tokens: TokenCount(Number::from(3_u8)),
            output_tokens: TokenCount(Number::from(5_u8)),
            ..ir::DecisionUsage::default()
        },
        extra: Extra::new(),
    }
}

fn systemone_body() -> ProviderRequestWire {
    ProviderRequestWire::SystemOne(SystemOneRequestWire::from(&systemone_request()))
}

fn openai_body() -> ProviderRequestWire {
    ProviderRequestWire::OpenAi(OpenAiRequestWire {
        model: String::from("model-1"),
        input: InputWire::Text(String::from("state")),
        questions: vec![QuestionWire::Predicate {
            name: Some(String::from("first")),
            instructions: String::from("Is it true?"),
        }],
        safety_identifier: None,
    })
}

fn call<'logging>(
    provider: &str,
    request: RequestFormat,
    body: ProviderRequestWire,
    logging: Option<&'logging Mutex<LoggingState>>,
) -> DecisionsCall<'logging> {
    DecisionsCall {
        model: String::from("model-1"),
        requested_model: String::from("requested/model"),
        custom_llm_provider: provider.to_owned(),
        provider_config: ProviderConfig::Typesafe(Box::new(TypeSafeDecisionsConfig)),
        request,
        ir_request: ir_request(),
        body,
        api_base: String::from("http://127.0.0.1:9"),
        api_key: Some(SecretString::from("test-key".to_owned())),
        logging_obj: logging,
        headers: Headers::new(),
        timeout: None,
        litellm_params: Extra::new(),
    }
}

fn systemone_call<'logging>(
    provider: &str,
    logging: Option<&'logging Mutex<LoggingState>>,
) -> DecisionsCall<'logging> {
    let request = systemone_request();
    call(
        provider,
        RequestFormat::SystemOne(request.body.clone()),
        systemone_body(),
        logging,
    )
}

fn openai_call<'logging>(
    provider: &str,
    logging: Option<&'logging Mutex<LoggingState>>,
) -> DecisionsCall<'logging> {
    let request = openai_request();
    call(
        provider,
        RequestFormat::OpenAi(request.body.clone()),
        openai_body(),
        logging,
    )
}

fn real_clients() -> HttpClients {
    let proxy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
        .expect("the policy resolves");
    let tls = TlsPolicy {
        verification: Verification::Enabled,
        ca_file: None,
        certificate: None,
        http2: false,
        ecdh_curve: None,
        user_agent: "betterlite-test/0".to_owned(),
    };
    HttpClients::new(tls, Arc::new(proxy))
}

// The failure a blocking send raises when its deadline passes, taken from a real client
fn timeout_failure() -> RequestFailure {
    let serving = serve(reply("200 OK", &[], "{}"), Duration::from_millis(300))
        .expect("the slow server starts");
    let mut clients = real_clients();
    let client = DecisionsClients::select_sync(&mut clients, &Params::new())
        .expect("the blocking client builds");
    let request = DecisionsRequest {
        url: Url::parse(&format!("http://{}/slow", serving.address))
            .expect("the fixture url parses"),
        headers: HeaderMap::new(),
        body: DecisionsBody::Json(Vec::new()),
        timeout: Some(Duration::from_millis(20)),
    };
    let failure = client
        .send_blocking(request)
        .expect_err("the deadline ends the send");
    drop(serving.finish());
    failure
}

async fn async_timeout_failure() -> RequestFailure {
    let serving = serve(reply("200 OK", &[], "{}"), Duration::from_millis(300))
        .expect("the slow server starts");
    let mut clients = real_clients();
    let client =
        DecisionsClients::select_async(&mut clients, &Params::new()).expect("the client builds");
    let request = DecisionsRequest {
        url: Url::parse(&format!("http://{}/slow", serving.address))
            .expect("the fixture url parses"),
        headers: HeaderMap::new(),
        body: DecisionsBody::Json(Vec::new()),
        timeout: Some(Duration::from_millis(20)),
    };
    let failure = client
        .send(request)
        .await
        .expect_err("the deadline ends the send");
    drop(serving.finish());
    failure
}

// A real connect failure: the listener closes before the client connects
fn connection_failure() -> RequestFailure {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port is free");
    let port = listener.local_addr().expect("the port is known").port();
    drop(listener);
    let mut clients = real_clients();
    let client = DecisionsClients::select_sync(&mut clients, &Params::new())
        .expect("the blocking client builds");
    let request = DecisionsRequest {
        url: Url::parse(&format!("http://127.0.0.1:{port}/v1/systemone"))
            .expect("the fixture url parses"),
        headers: HeaderMap::new(),
        body: DecisionsBody::Json(Vec::new()),
        timeout: Some(Duration::from_millis(500)),
    };
    client
        .send_blocking(request)
        .expect_err("nothing listens on the closed port")
}

fn class_of(error: &DecisionsSendError) -> &ErrorClass {
    match error {
        DecisionsSendError::Mapped { class, .. } => class,
        DecisionsSendError::Format(_) => panic!("the call was expected to fail as a mapped error"),
    }
}

fn answer_count(error: ResponseConversionError) -> (usize, usize) {
    match error {
        ResponseConversionError::AnswerCount { answers, questions } => (answers, questions),
        other => panic!("expected an answer count error, got {other}"),
    }
}

// request_prep: validate_environment, url, the body copy and the optional pre_call, in that order

#[test]
fn request_prep_runs_validate_url_and_pre_call_in_order() {
    let mut fixture = Fixture::new();
    let logging = Mutex::new(logging_state());
    let mut decisions = systemone_call("typesafe", Some(&logging));
    decisions
        .headers
        .insert("X-Custom".to_owned(), "one".to_owned());
    let output = fixture.send(&decisions).expect("the call answers");
    assert!(matches!(output, DecisionsOutput::SystemOne(_)));
    assert_eq!(
        fixture.recording.events.recorded(),
        expected(&[
            "validate",
            "url",
            "log",
            "sign",
            "select-sync",
            "send",
            "transform",
        ])
    );
    let validations = fixture.recording.seen_validations();
    assert_eq!(validations.len(), 1);
    assert_eq!(validations[0].model, "model-1");
    assert_eq!(validations[0].api_key.as_deref(), Some("test-key"));
    assert_eq!(
        validations[0].headers,
        vec![(String::from("X-Custom"), String::from("one"))]
    );
}

#[test]
fn request_prep_builds_url_from_api_base_and_model() {
    let mut fixture = Fixture::new();
    let decisions = systemone_call("typesafe", None);
    fixture.send(&decisions).expect("the call answers");
    assert_eq!(
        fixture.recording.seen_urls(),
        vec![(String::from("http://127.0.0.1:9"), String::from("model-1"))]
    );
}

#[test]
fn request_prep_skips_logging_without_a_logging_object() {
    let mut fixture = Fixture::new();
    let decisions = systemone_call("typesafe", None);
    fixture.send(&decisions).expect("the call answers");
    let events = fixture.recording.events.recorded();
    assert!(!events.contains(&String::from("log")));
    assert_eq!(fixture.wire.requests().len(), 1);
}

#[test]
fn request_prep_logs_the_unsigned_headers_and_body_copy() {
    let mut fixture = Fixture::new();
    fixture.config.signature = Some(String::from("sig"));
    let logging = Mutex::new(logging_state());
    let decisions = systemone_call("typesafe", Some(&logging));
    fixture.send(&decisions).expect("the call answers");
    let signed = &fixture.recording.seen_signs()[0];
    assert!(signed.headers.contains(&(
        String::from("Content-Type"),
        String::from("application/json")
    )));
    assert!(signed.headers.contains(&(
        String::from("Authorization"),
        String::from("Bearer test-key")
    )));
    let seen = fixture
        .recording
        .seen_validations()
        .first()
        .cloned()
        .expect("the environment was validated");
    assert!(
        seen.headers.is_empty(),
        "pre_call must see the call's own headers, not the signed ones"
    );
}

// send_behavior: one POST of the transformed body with the validated headers

#[test]
fn send_behavior_posts_transformed_body_with_bearer_headers() {
    let serving = serve(reply("200 OK", &[], SYSTEMONE_ANSWER), Duration::default())
        .expect("the mock server starts");
    let api_base = format!("http://{}", serving.address);
    let catalog = ModelCatalog::default();
    let known = BTreeSet::new();
    let custom = BTreeSet::new();
    let environment = BTreeMap::new();
    let resolution = ResolutionContext {
        catalog: &catalog,
        known_model_names: &known,
        custom_providers: &custom,
        environment: &environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let key = SecretString::from("test-key".to_owned());
    let logging = Mutex::new(logging_state());
    let questions =
        serde_json::from_str(r#"{"first":{"type":"noul","instructions":"Is it true?"}}"#)
            .expect("the questions parse");
    let decisions = prepare_call(PrepareCall {
        model: "model-1",
        fields: RequestFields::SystemOne(SystemOneRequestFields {
            state: Some(Json::Text(String::from("state"))),
            questions: Some(questions),
        }),
        safety_identifier: None,
        api_key: Some(&key),
        api_base: Some(&api_base),
        timeout: None,
        custom_llm_provider: Some("typesafe"),
        extra_headers: None,
        kwargs: Extra::new(),
        global_drop_params: false,
        globals: LiteLlmGlobals {
            api_base: None,
            api_key: None,
            openai_key: None,
            organization: None,
        },
        azure_ai: AzureAiConfig {
            complete_url: Box::new(|api_base: &str, _model: &str| api_base.to_owned()),
        },
        resolution: &resolution,
        logging_obj: Some(&logging),
    })
    .expect("the call prepares");
    let expected_body = serde_json::to_string(&decisions.body).expect("the wire serializes");
    let mut clients = real_clients();
    let log = LogEnv::for_events(&Events::new());
    let env = log.env();
    let mut sender = DecisionsSender::new(&mut clients, &env);
    let output = sender.send(&decisions).expect("the mock answers");
    match output {
        DecisionsOutput::SystemOne(response) => {
            assert_eq!(answer_names(&response), vec![String::from("first")]);
            let usage = response.usage.expect("the conversion sets usage");
            assert_eq!(usage.input_tokens.0, Number::from(3_u8));
        }
        DecisionsOutput::OpenAi(_) => {
            panic!("a SystemOne request answers with a SystemOne response")
        }
    }
    let request = serving.finish().expect("the server saw the request");
    assert_eq!(request.line, "POST /v1/systemone HTTP/1.1");
    assert!(
        request
            .head
            .to_lowercase()
            .contains("authorization: bearer test-key")
    );
    assert!(
        request
            .head
            .to_lowercase()
            .contains("content-type: application/json")
    );
    assert_eq!(request.body, expected_body);
}

#[tokio::test]
async fn send_behavior_async_answers_and_selects_the_provider_client() {
    let mut fixture = Fixture::new();
    let decisions = systemone_call("typesafe", None);
    let output = fixture.asend(&decisions).await.expect("the call answers");
    assert!(matches!(output, DecisionsOutput::SystemOne(_)));
    let selections = fixture.clients.selections();
    assert_eq!(selections.len(), 1);
    assert_eq!(selections[0].0, Mode::Asynchronous);
    assert_eq!(
        selections[0].1.get("custom_llm_provider"),
        Some(&JsonValue::Text(String::from("typesafe")))
    );
}

#[test]
fn send_behavior_sync_selects_the_default_client() {
    let mut fixture = Fixture::new();
    let decisions = systemone_call("typesafe", None);
    fixture.send(&decisions).expect("the call answers");
    let selections = fixture.clients.selections();
    assert_eq!(selections.len(), 1);
    assert_eq!(selections[0].0, Mode::Sync);
    assert!(selections[0].1.is_empty());
}

#[test]
fn send_behavior_passes_the_timeout_and_posts_once() {
    let mut fixture = Fixture::new();
    let mut decisions = systemone_call("typesafe", None);
    decisions.timeout = Some(Duration::from_secs(2));
    fixture.send(&decisions).expect("the call answers");
    let requests = fixture.wire.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].timeout, Some(Duration::from_secs(2)));
    match &requests[0].body {
        DecisionsBody::Json(bytes) => {
            let sent = serde_json::to_string(&decisions.body).expect("the wire serializes");
            assert_eq!(bytes, sent.as_bytes());
        }
        DecisionsBody::Signed(_) => panic!("an unsigned config sends JSON"),
    }
}

#[test]
fn send_behavior_sends_the_signed_headers() {
    let mut fixture = Fixture::new();
    fixture.config.signature = Some(String::from("sig-1"));
    let decisions = systemone_call("typesafe", None);
    fixture.send(&decisions).expect("the call answers");
    let requests = fixture.wire.requests();
    assert_eq!(
        requests[0].headers.get("x-signature"),
        Some(&HeaderValue::from_static("sig-1"))
    );
    assert_eq!(
        requests[0].headers.get("content-type"),
        Some(&HeaderValue::from_static("application/json"))
    );
}

// signing: content when the config signed a body, JSON when it did not, and the async AWS split

#[test]
fn signing_signed_body_is_sent_as_content() {
    let mut fixture = Fixture::new();
    fixture.config.signed_body = Some(b"{\"signed\":true}".to_vec());
    let decisions = systemone_call("typesafe", None);
    fixture.send(&decisions).expect("the call answers");
    let requests = fixture.wire.requests();
    match &requests[0].body {
        DecisionsBody::Signed(bytes) => assert_eq!(bytes, b"{\"signed\":true}"),
        DecisionsBody::Json(_) => panic!("a signed body must be sent as content"),
    }
}

#[test]
fn signing_unsigned_body_is_sent_as_json() {
    let mut fixture = Fixture::new();
    let decisions = systemone_call("typesafe", None);
    fixture.send(&decisions).expect("the call answers");
    let requests = fixture.wire.requests();
    match &requests[0].body {
        DecisionsBody::Json(bytes) => {
            let sent = serde_json::to_string(&decisions.body).expect("the wire serializes");
            assert_eq!(bytes, sent.as_bytes());
        }
        DecisionsBody::Signed(_) => panic!("an unsigned result must be sent as JSON"),
    }
}

#[test]
fn signing_sync_signs_inline_even_for_aws() {
    let mut fixture = Fixture::new();
    fixture.config.aws = true;
    let decisions = systemone_call("strands_decider", None);
    let caller = thread::current().id();
    fixture.send(&decisions).expect("the call answers");
    assert_eq!(fixture.recording.sign_threads(), vec![caller]);
}

#[tokio::test]
async fn signing_async_signs_inline_without_aws() {
    let mut fixture = Fixture::new();
    let decisions = systemone_call("typesafe", None);
    let caller = thread::current().id();
    fixture.asend(&decisions).await.expect("the call answers");
    assert_eq!(fixture.recording.sign_threads(), vec![caller]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn signing_async_signs_off_the_request_task_with_aws() {
    let mut fixture = Fixture::new();
    fixture.config.aws = true;
    let decisions = systemone_call("strands_decider", None);
    let caller = thread::current().id();
    fixture.asend(&decisions).await.expect("the call answers");
    let threads = fixture.recording.sign_threads();
    assert_eq!(threads.len(), 1);
    assert_ne!(threads[0], caller);
}

#[test]
fn signing_context_carries_the_call_inputs() {
    let mut fixture = Fixture::new();
    let decisions = systemone_call("typesafe", None);
    fixture.send(&decisions).expect("the call answers");
    let signed = &fixture.recording.seen_signs()[0];
    assert_eq!(signed.url, "http://127.0.0.1:9/v1/systemone");
    assert_eq!(signed.api_base, "http://127.0.0.1:9");
    assert_eq!(signed.api_key.as_deref(), Some("test-key"));
    assert!(signed.params_empty);
}

// format_response: the shape follows the request, and the hidden params follow the config's cost

#[test]
fn format_response_systemone_keeps_answers_and_usage() {
    let fixture = Fixture::new();
    let decisions = systemone_call("typesafe", None);
    let output = format_response(
        &fixture.config,
        &decisions,
        &ir_response(Some("upstream-model")),
    )
    .expect("the response formats");
    match output {
        DecisionsOutput::SystemOne(response) => {
            assert_eq!(response.model.as_deref(), Some("upstream-model"));
            assert!(matches!(
                response.answers.get("first"),
                Some(systemone::DecisionAnswer::Noul { noul, .. })
                    if *noul == Number::from_f64(0.9).expect("a finite number")
            ));
            let usage = response.usage.expect("the conversion sets usage");
            assert_eq!(usage.input_tokens.0, Number::from(3_u8));
            assert_eq!(usage.output_tokens.0, Number::from(5_u8));
        }
        DecisionsOutput::OpenAi(_) => {
            panic!("a SystemOne request answers with a SystemOne response")
        }
    }
}

#[test]
fn format_response_systemone_rejects_a_different_answer_count() {
    let fixture = Fixture::new();
    let decisions = systemone_call("typesafe", None);
    let mut response = ir_response(Some("model-1"));
    response.answers.push(ir::DecisionAnswer::Refusal);
    let error =
        format_response(&fixture.config, &decisions, &response).expect_err("the counts differ");
    assert_eq!(answer_count(error), (2, 1));
}

#[test]
fn format_response_openai_uses_the_requested_model() {
    let fixture = Fixture::new();
    let decisions = openai_call("typesafe", None);
    let output =
        format_response(&fixture.config, &decisions, &ir_response(None)).expect("it formats");
    match output {
        DecisionsOutput::OpenAi(response) => assert_eq!(response.model, "requested/model"),
        DecisionsOutput::SystemOne(_) => {
            panic!("an OpenAI request answers with an OpenAI response")
        }
    }
    let named = format_response(
        &fixture.config,
        &decisions,
        &ir_response(Some("provider-model")),
    )
    .expect("the response formats");
    match named {
        DecisionsOutput::OpenAi(response) => assert_eq!(response.model, "provider-model"),
        DecisionsOutput::SystemOne(_) => {
            panic!("an OpenAI request answers with an OpenAI response")
        }
    }
}

#[test]
fn format_response_openai_rejects_a_different_answer_count() {
    let fixture = Fixture::new();
    let decisions = openai_call("typesafe", None);
    let mut response = ir_response(Some("model-1"));
    response.answers.push(ir::DecisionAnswer::Refusal);
    let error =
        format_response(&fixture.config, &decisions, &response).expect_err("the counts differ");
    assert_eq!(answer_count(error), (2, 1));
}

#[test]
fn format_response_hidden_params_without_cost() {
    let fixture = Fixture::new();
    let decisions = systemone_call("typesafe", None);
    let output = format_response(&fixture.config, &decisions, &ir_response(Some("model-1")))
        .expect("the response formats");
    let response = match output {
        DecisionsOutput::SystemOne(response) => response,
        DecisionsOutput::OpenAi(_) => {
            panic!("a SystemOne request answers with a SystemOne response")
        }
    };
    assert_eq!(
        response.hidden_params.get("model"),
        Some(&Json::Text(String::from("typesafe/model-1")))
    );
    assert_eq!(
        response.hidden_params.get("provider_response_model"),
        Some(&Json::Text(String::from("typesafe/model-1")))
    );
    assert_eq!(
        response.hidden_params.get("custom_llm_provider"),
        Some(&Json::Text(String::from("typesafe")))
    );
    assert!(!response.hidden_params.contains_key("additional_headers"));
    let serialized = serde_json::to_string(&response).expect("the response serializes");
    assert!(!serialized.contains("hidden_params"));
    assert!(!serialized.contains(RESPONSE_COST_HEADER));
}

#[test]
fn format_response_hidden_params_with_cost() {
    let fixture = Fixture::new();
    let mut config = TestConfig::new(&fixture.recording);
    config.cost = Some(Decimal::new(25, 4));
    let decisions = systemone_call("typesafe", None);
    let output = format_response(&config, &decisions, &ir_response(Some("model-1")))
        .expect("the response formats");
    let response = match output {
        DecisionsOutput::SystemOne(response) => response,
        DecisionsOutput::OpenAi(_) => {
            panic!("a SystemOne request answers with a SystemOne response")
        }
    };
    let headers = response
        .hidden_params
        .get("additional_headers")
        .expect("the cost header is set");
    match headers {
        Json::Object(entries) => {
            assert_eq!(entries.len(), 1);
            match entries.get(RESPONSE_COST_HEADER) {
                Some(Json::Number(cost)) => assert_eq!(cost.as_f64(), Some(0.0025)),
                other => panic!("the cost header holds a number, got {other:?}"),
            }
        }
        _ => panic!("additional_headers is a map"),
    }
    let serialized = serde_json::to_string(&response).expect("the response serializes");
    assert!(!serialized.contains("additional_headers"));
    assert!(!serialized.contains(RESPONSE_COST_HEADER));
}

// error_mapping: the provider's class, the synthesized status branch and the validation branch

#[test]
fn error_mapping_rate_limit_keeps_the_status_and_provider_message() {
    let mut fixture = Fixture::new();
    fixture.wire.answering(429, b"rate limited");
    fixture.wire.with_header("retry-after", "2");
    let decisions = systemone_call("typesafe", None);
    let error = fixture.send(&decisions).expect_err("the provider refused");
    assert!(matches!(class_of(&error), ErrorClass::RateLimit(_)));
    assert_eq!(
        error.message(),
        "litellm.RateLimitError: TypesafeException - rate limited"
    );
    assert_eq!(class_of(&error).metadata().status_code, 429);
    let seen = fixture.recording.seen_errors();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, "rate limited");
    assert_eq!(seen[0].1, 429);
    assert_eq!(seen[0].2.get("retry-after"), Some(&String::from("2")));
    assert_eq!(fixture.wire.requests().len(), 1);
}

#[test]
fn error_mapping_authentication() {
    let mut fixture = Fixture::new();
    fixture.wire.answering(401, b"bad key");
    let decisions = systemone_call("typesafe", None);
    let error = fixture.send(&decisions).expect_err("the provider refused");
    assert!(matches!(class_of(&error), ErrorClass::Authentication(_)));
    assert_eq!(
        error.message(),
        "litellm.AuthenticationError: TypesafeException - bad key"
    );
    assert_eq!(class_of(&error).metadata().status_code, 401);
}

#[test]
fn error_mapping_synthesized_connection_failure_is_an_api_connection_error() {
    let fixture = Fixture::new();
    let call = systemone_call("typesafe", None);
    let failure = connection_failure();
    let expected_text = failure.to_string();
    let error = send_failure(
        &fixture.config,
        &call,
        &fixture.log.redactor,
        failure,
        ClientKind::Sync,
        Duration::default(),
    );
    assert!(matches!(class_of(&error), ErrorClass::ApiConnection(_)));
    assert_eq!(
        error.message(),
        format!("litellm.APIConnectionError: TypesafeException - {expected_text}")
    );
    assert_eq!(class_of(&error).metadata().status_code, 500);
    assert_eq!(fixture.recording.seen_errors()[0].1, 500);
}

#[test]
fn error_mapping_synthesized_labels_upper_case_only_the_first_character() {
    let fixture = Fixture::new();
    let cases = [
        ("typesafe", "TypesafeException"),
        ("hosted_vllm", "Hosted_vllmException"),
        ("strands_decider", "Strands_deciderException"),
    ];
    for (provider, label) in cases {
        let call = systemone_call(provider, None);
        let error = provider_error(
            &call,
            &fixture.log.redactor,
            BaseLlmException {
                status_code: 500,
                message: String::from("boom"),
                headers: BTreeMap::new(),
                status_code_is_synthesized: true,
            },
        );
        assert_eq!(
            error.message(),
            format!("litellm.APIConnectionError: {label} - boom")
        );
        assert_eq!(class_of(&error).metadata().status_code, 500);
    }
}

#[test]
fn error_mapping_sync_timeout_keeps_the_handler_message() {
    let mut fixture = Fixture::failing(timeout_failure());
    let mut decisions = systemone_call("typesafe", None);
    decisions.timeout = Some(Duration::from_millis(20));
    let error = fixture.send(&decisions).expect_err("the send timed out");
    assert!(matches!(class_of(&error), ErrorClass::Timeout(_)));
    assert_eq!(
        error.message(),
        "litellm.Timeout: Connection timed out after 0.02 seconds."
    );
    assert!(
        fixture.recording.seen_errors().is_empty(),
        "the client raises litellm.Timeout itself, so no provider class is asked for"
    );
}

#[tokio::test]
async fn error_mapping_async_timeout_keeps_the_handler_message() {
    let failure = async_timeout_failure().await;
    let mut fixture = Fixture::failing(failure);
    let mut decisions = systemone_call("typesafe", None);
    decisions.timeout = Some(Duration::from_millis(20));
    let error = fixture
        .asend(&decisions)
        .await
        .expect_err("the send timed out");
    assert!(matches!(class_of(&error), ErrorClass::Timeout(_)));
    assert!(
        error
            .message()
            .starts_with("litellm.Timeout: Connection timed out. Timeout passed=0.02, time taken="),
        "the async handler names the time taken, got {}",
        error.message()
    );
    assert!(fixture.recording.seen_errors().is_empty());
}

#[test]
fn error_mapping_invalid_answer_goes_through_the_status_mapping() {
    let mut fixture = Fixture::new();
    *fixture
        .config
        .transform
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Transform::Invalid(String::from(
        "Decisions provider 'typesafe' returned an unexpected response: bad answers",
    ));
    let decisions = systemone_call("typesafe", None);
    let error = fixture
        .send(&decisions)
        .expect_err("the answer does not validate");
    assert!(matches!(class_of(&error), ErrorClass::InternalServer(_)));
    assert_eq!(
        error.message(),
        "litellm.InternalServerError: TypesafeException - Decisions provider 'typesafe' returned an unexpected response: bad answers"
    );
    assert_eq!(
        class_of(&error).metadata().error_type,
        WireField::Text(String::from("internal_server_error"))
    );
    assert!(
        fixture.recording.seen_errors().is_empty(),
        "the transform raises its own class, not the transport conversion"
    );
}

#[test]
fn error_mapping_malformed_answer_is_an_api_connection_error() {
    let mut fixture = Fixture::new();
    *fixture
        .config
        .transform
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Transform::Malformed;
    let decisions = systemone_call("typesafe", None);
    let error = fixture
        .send(&decisions)
        .expect_err("the answer does not parse");
    assert!(matches!(class_of(&error), ErrorClass::ApiConnection(_)));
    assert!(
        error
            .message()
            .starts_with("litellm.APIConnectionError: TypesafeException - "),
        "a malformed answer is a plain exception, mapped by exception_type"
    );
}

#[test]
fn error_mapping_client_selection_failure_is_an_api_connection_error() {
    let mut fixture = Fixture::new();
    let wire = fixture.wire.clone();
    fixture.clients = FakeClients::failing(
        &fixture.recording,
        wire,
        ClientError::UnknownProvider {
            provider: String::from("nope"),
        },
    );
    let decisions = systemone_call("typesafe", None);
    let error = fixture
        .send(&decisions)
        .expect_err("the client cannot be built");
    assert!(matches!(class_of(&error), ErrorClass::ApiConnection(_)));
    assert!(fixture.wire.requests().is_empty(), "no request is sent");
}
