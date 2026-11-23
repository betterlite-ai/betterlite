use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_config::types::ProxyTrust;
use betterlite_http::interactions::base::clients::HttpClients;
use betterlite_http::interactions::base::{ClientError, Params, RequestFailure};
use betterlite_http::proxy::UpstreamResponse;
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
use betterlite_models::decisions::systemone;
use betterlite_models::decisions::{DecisionsJson, Extra, Json, TokenCount, ir, openai};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::decisions::call::DecisionsCall;
use betterlite_openai::decisions::call::azure::AzureAiConfig;
use betterlite_openai::decisions::call::fields::{
    RequestFields, RequestFormat, SystemOneRequestFields,
};
use betterlite_openai::decisions::call::provider::ProviderConfig;
use betterlite_openai::decisions::call::{PrepareCall, prepare_call};
use betterlite_openai::decisions::openai::request::wire::{
    InputWire, QuestionWire, RequestWire as OpenAiRequestWire,
};
use betterlite_openai::decisions::transformation::provider_config::{
    DecisionsProviderConfig, ParseResponseError, SigningContext,
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
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;

use super::super::clients::{
    Answer, DecisionsAsyncTransport, DecisionsRequest, DecisionsSyncTransport,
};
use super::super::error::DecisionsSendError;
use super::super::format::DecisionsOutput;
use super::super::send::DecisionsSender;
use super::super::signing::{SignFuture, SignTask, SigningSeam};

pub(super) const SYSTEMONE_ANSWER: &str = r#"{"model":"model-1","answers":{"first":{"type":"noul","noul":0.9}},"usage":{"input_tokens":3,"output_tokens":5}}"#;

#[derive(Clone, Default)]
pub(super) struct Events(Arc<Mutex<Vec<String>>>);

impl Events {
    pub(super) fn new() -> Self {
        Self::default()
    }

    fn push(&self, name: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(name.to_owned());
    }

    pub(super) fn recorded(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(super) fn expected(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

pub(super) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

type CallLogger = Box<dyn Fn(&Json) -> Result<(), PrePostError> + Send + Sync>;

// The logging environment a pre_call needs, with a logger fn that records what it was handed
pub(super) struct LogEnv {
    pub(super) redactor: Redactor,
    sink: Sink,
    now: chrono::DateTime<chrono::FixedOffset>,
    logger: CallLogger,
}

impl LogEnv {
    pub(super) fn for_events(events: &Events) -> Self {
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

    pub(super) fn env(&self) -> PrePostEnv<'_> {
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

pub(super) fn logging_state() -> LoggingState {
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
pub(super) struct Validated {
    pub(super) headers: Vec<(String, String)>,
    pub(super) model: String,
    pub(super) api_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SignSeen {
    pub(super) headers: Vec<(String, String)>,
    pub(super) url: String,
    pub(super) api_base: String,
    pub(super) api_key: Option<String>,
    pub(super) params: Extra,
}

#[derive(Default)]
pub(super) struct Recording {
    pub(super) events: Events,
    validations: Mutex<Vec<Validated>>,
    urls: Mutex<Vec<(String, String)>>,
    signs: Mutex<Vec<SignSeen>>,
    pub(super) errors: Mutex<Vec<(String, u16, Headers)>>,
}

impl Recording {
    pub(super) fn seen_validations(&self) -> Vec<Validated> {
        self.validations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn seen_urls(&self) -> Vec<(String, String)> {
        self.urls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn seen_signs(&self) -> Vec<SignSeen> {
        self.signs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn seen_errors(&self) -> Vec<(String, u16, Headers)> {
        self.errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(super) enum Transform {
    Answer(Box<ir::DecisionResponse>),
    Invalid(String),
    Malformed,
}

// The fake config answers every provider call the send stage makes and records it
pub(super) struct TestConfig {
    recording: Arc<Recording>,
    pub(super) transform: Mutex<Transform>,
    pub(super) cost: Option<Decimal>,
    pub(super) aws: bool,
    pub(super) signature: Option<String>,
    pub(super) signed_body: Option<Vec<u8>>,
}

impl TestConfig {
    pub(super) fn new(recording: &Arc<Recording>) -> Self {
        Self {
            recording: Arc::clone(recording),
            transform: Mutex::new(Transform::Answer(Box::new(ir_response(Some("model-1"))))),
            cost: None,
            aws: false,
            signature: None,
            signed_body: None,
        }
    }
}

impl DecisionsProviderConfig for TestConfig {
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
            .signs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(SignSeen {
                headers: pairs(&headers),
                url: context.url.to_owned(),
                api_base: context.api_base.to_owned(),
                api_key: context.api_key.map(|key| key.expose_secret().to_owned()),
                params: context.litellm_params.clone(),
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
        _raw_response: &[u8],
        _request: &ir::DecisionRequest,
    ) -> Result<ir::DecisionResponse, ParseResponseError> {
        self.recording.events.push("transform");
        match &*self
            .transform
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
        {
            Transform::Answer(response) => Ok(response.as_ref().clone()),
            Transform::Invalid(message) => Err(ParseResponseError::Invalid(Box::new(
                provider_error_of(message, &Headers::new()),
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
        provider_error_of_with_status(error_message, status_code, headers)
    }
}

fn provider_error_of(message: &str, headers: &Headers) -> ProviderError {
    provider_error_of_with_status(message, 500, headers)
}

fn provider_error_of_with_status(
    message: &str,
    status_code: u16,
    headers: &Headers,
) -> ProviderError {
    ProviderError::from(ProviderErrorParts {
        status_code,
        message: message.to_owned(),
        headers: Some(headers.clone().into_iter().collect()),
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

impl FakeAnswer {
    fn new(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: HeaderMap::new(),
            body: body.as_bytes().to_vec(),
        }
    }
}

// The wire a decisions call sends through: it records the request and answers or fails with the fixture's choice
#[derive(Clone)]
pub(super) struct FakeWire {
    recording: Arc<Recording>,
    answer: Arc<Mutex<FakeAnswer>>,
    failure: Arc<Mutex<Option<RequestFailure>>>,
    requests: Arc<Mutex<Vec<DecisionsRequest>>>,
}

impl FakeWire {
    fn new(recording: &Arc<Recording>, answer: FakeAnswer) -> Self {
        let shared = Arc::clone(recording);
        Self {
            recording: shared,
            answer: Arc::new(Mutex::new(answer)),
            failure: Arc::default(),
            requests: Arc::default(),
        }
    }

    fn failing(recording: &Arc<Recording>, failure: RequestFailure) -> Self {
        let wire = Self::new(recording, FakeAnswer::new(200, "{}"));
        *wire.failure.lock().unwrap_or_else(PoisonError::into_inner) = Some(failure);
        wire
    }

    pub(super) fn answering(&self, status: u16, body: &[u8]) {
        let mut answer = self.answer.lock().unwrap_or_else(PoisonError::into_inner);
        answer.status = status;
        answer.body = body.to_vec();
    }

    pub(super) fn with_header(&self, name: &'static str, value: &'static str) {
        self.answer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .headers
            .append(
                HeaderName::from_static(name),
                HeaderValue::from_static(value),
            );
    }

    pub(super) fn requests(&self) -> MutexGuard<'_, Vec<DecisionsRequest>> {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Sync,
    Asynchronous,
}

pub(super) struct FakeClients {
    pub(super) recording: Arc<Recording>,
    wire: FakeWire,
    failure: Option<ClientError>,
    selections: Mutex<Vec<(Mode, Params)>>,
}

impl FakeClients {
    fn from_wire(recording: &Arc<Recording>, wire: FakeWire) -> Self {
        Self {
            recording: Arc::clone(recording),
            wire,
            failure: None,
            selections: Mutex::default(),
        }
    }

    pub(super) fn failing(
        recording: &Arc<Recording>,
        wire: FakeWire,
        failure: ClientError,
    ) -> Self {
        let mut clients = Self::from_wire(recording, wire);
        clients.failure = Some(failure);
        clients
    }

    pub(super) fn selections(&self) -> Vec<(Mode, Params)> {
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

impl DecisionsSyncTransport for FakeClients {
    fn send_sync(
        &mut self,
        params: &Params,
        request: DecisionsRequest,
    ) -> Result<Answer, ClientError> {
        let wire = self.selected(Mode::Sync, params)?;
        Ok(wire.record(request))
    }
}

// The seam a case injects: it records the off-loop route and signs on the task that called it
pub(super) struct RecordingSigning {
    events: Events,
}

impl SigningSeam for RecordingSigning {
    fn sign<'seam>(&'seam self, off_loop: bool, sign: SignTask<'seam>) -> SignFuture<'seam> {
        if off_loop {
            self.events.push("off-loop");
        }
        Box::pin(ready(sign()))
    }
}

impl DecisionsAsyncTransport for FakeClients {
    fn send_async(
        &mut self,
        params: &Params,
        request: DecisionsRequest,
    ) -> Result<impl Future<Output = Answer> + Send, ClientError> {
        let wire = self.selected(Mode::Asynchronous, params)?;
        Ok(ready(wire.record(request)))
    }
}

// One test's config, logging environment, clients and wire, so a case only builds its call
pub(super) struct Fixture {
    pub(super) config: TestConfig,
    pub(super) log: LogEnv,
    pub(super) clients: FakeClients,
    pub(super) wire: FakeWire,
    pub(super) recording: Arc<Recording>,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let recording = Arc::new(Recording::default());
        let wire = FakeWire::new(&recording, FakeAnswer::new(200, SYSTEMONE_ANSWER));
        let clients = FakeClients::from_wire(&recording, wire.clone());
        Self {
            config: TestConfig::new(&recording),
            log: LogEnv::for_events(&recording.events),
            clients,
            wire,
            recording,
        }
    }

    pub(super) fn failing(failure: RequestFailure) -> Self {
        let mut fixture = Self::new();
        let wire = FakeWire::failing(&fixture.recording, failure);
        fixture.clients = FakeClients::from_wire(&fixture.recording, wire.clone());
        fixture.wire = wire;
        fixture
    }

    pub(super) fn send(
        &mut self,
        call: &DecisionsCall<'_>,
    ) -> Result<DecisionsOutput, DecisionsSendError> {
        DecisionsSender::new(&mut self.clients, &self.log.env()).send_with(call, &self.config)
    }

    pub(super) async fn asend(
        &mut self,
        call: &DecisionsCall<'_>,
    ) -> Result<DecisionsOutput, DecisionsSendError> {
        let signing = RecordingSigning {
            events: self.recording.events.clone(),
        };
        DecisionsSender::new(&mut self.clients, &self.log.env())
            .asend_with(call, &self.config, signing)
            .await
    }
}

fn pairs(headers: &Headers) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

pub(super) fn answer_names(response: &systemone::DecisionsResponse) -> Vec<String> {
    response.answers.keys().cloned().collect()
}

pub(super) fn systemone_request() -> systemone::DecisionsRequest {
    serde_json::from_str(
        r#"{"model":"model-1","state":"state","questions":{"first":{"type":"noul","instructions":"Is it true?"}}}"#,
    )
    .expect("the request fixture parses")
}

pub(super) fn openai_request() -> openai::DecisionRequest {
    serde_json::from_str(
        r#"{"model":"model-1","input":"state","questions":[{"type":"predicate","name":"first","instructions":"Is it true?"}]}"#,
    )
    .expect("the request fixture parses")
}

fn predicate_question() -> ir::DecisionQuestion {
    ir::DecisionQuestion::Predicate(ir::PredicateQuestion {
        name: Some(String::from("first")),
        instructions: Some(DecisionsJson::Text(String::from("Is it true?"))),
        criteria: None,
        extra: Extra::new(),
    })
}

fn predicate_answer() -> ir::DecisionAnswer {
    ir::DecisionAnswer::Predicate {
        probability: Number::from_f64(0.9).expect("a finite probability"),
        extra: Extra::new(),
    }
}

fn ir_usage() -> ir::DecisionUsage {
    ir::DecisionUsage {
        input_tokens: TokenCount(Number::from(3_u8)),
        output_tokens: TokenCount(Number::from(5_u8)),
        cached_tokens: TokenCount::default(),
        cache_write_tokens: TokenCount::default(),
        reasoning_tokens: TokenCount::default(),
        extra: Extra::new(),
    }
}

pub(super) fn ir_request() -> ir::DecisionRequest {
    ir::DecisionRequest {
        input: ir::DecisionInput::State(DecisionsJson::Text(String::from("state"))),
        questions: vec![predicate_question()],
        safety_identifier: None,
    }
}

pub(super) fn ir_response(model: Option<&str>) -> ir::DecisionResponse {
    ir::DecisionResponse {
        model: model.map(str::to_owned),
        answers: vec![predicate_answer()],
        usage: ir_usage(),
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

// The two envelopes a Decisions call can carry, each with the provider wire its request serializes to
#[derive(Clone, Copy)]
enum Shape {
    SystemOne,
    OpenAi,
}

fn shaped_call<'logging>(
    provider: &str,
    shape: Shape,
    logging: Option<&'logging Mutex<LoggingState>>,
) -> DecisionsCall<'logging> {
    let (request, body) = match shape {
        Shape::SystemOne => {
            let request = systemone_request();
            (RequestFormat::SystemOne(request.body), systemone_body())
        }
        Shape::OpenAi => {
            let request = openai_request();
            (RequestFormat::OpenAi(request.body), openai_body())
        }
    };
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

pub(super) fn systemone_call<'logging>(
    provider: &str,
    logging: Option<&'logging Mutex<LoggingState>>,
) -> DecisionsCall<'logging> {
    shaped_call(provider, Shape::SystemOne, logging)
}

pub(super) fn openai_call<'logging>(
    provider: &str,
    logging: Option<&'logging Mutex<LoggingState>>,
) -> DecisionsCall<'logging> {
    shaped_call(provider, Shape::OpenAi, logging)
}

// The resolution inputs a real prepare_call reads, so a test only names the call it builds
#[derive(Default)]
pub(super) struct Environment {
    catalog: ModelCatalog,
    known: BTreeSet<String>,
    custom: BTreeSet<String>,
    variables: BTreeMap<String, String>,
}

impl Environment {
    fn context(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.known,
            custom_providers: &self.custom,
            environment: &self.variables,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
}

// The full prepare_call path through B3511's TypeSafe config, which the provider wire is built from
pub(super) fn typesafe_call<'call>(
    api_base: &'call str,
    key: &'call SecretString,
    logging: &'call Mutex<LoggingState>,
    environment: &'call Environment,
) -> DecisionsCall<'call> {
    let questions =
        serde_json::from_str(r#"{"first":{"type":"noul","instructions":"Is it true?"}}"#)
            .expect("the questions parse");
    let resolution = environment.context();
    prepare_call(PrepareCall {
        model: "model-1",
        fields: RequestFields::SystemOne(SystemOneRequestFields {
            state: Some(Json::Text(String::from("state"))),
            questions: Some(questions),
        }),
        safety_identifier: None,
        api_key: Some(key),
        api_base: Some(api_base),
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
            complete_url: Box::new(|base: &str, _model: &str| base.to_owned()),
        },
        resolution: &resolution,
        logging_obj: Some(logging),
    })
    .expect("the call prepares")
}

pub(super) fn real_clients() -> HttpClients {
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
