#![cfg(test)]

use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_error::error::ErrorClass;
use betterlite_http::interactions::base::{
    ClientError, ErrorClasses, Params, RequestFailure, RequestHeaders, ResponseHeaders,
};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::HeaderMap;
use indexmap::IndexMap;
use thiserror::Error;

use super::super::error::CharacterProviderFailure;
use super::super::{
    CharacterCallDeps, CharacterOutcome, CreateCharacterCall, DEFAULT_PROVIDER,
    DEFAULT_TIMEOUT_SECONDS, GetCharacterCall, avideo_create_character, avideo_get_character,
    video_create_character, video_get_character,
};
use super::support::{CALL_ID, CHARACTER_JSON, headers, video};
use crate::api::bfl::edit::media::SuppliedMedia;
use crate::api::video_character_transport::clients::{
    CharacterAsyncClient, CharacterClients, CharacterRequest, CharacterSyncClient,
};
use crate::api::video_character_transport::config::{
    CharacterConfig, CharacterContext, CharacterEnvironment,
};
use crate::api::video_character_transport::service::CharacterTransport;
use crate::api::video_records::CharacterObject;

#[derive(Debug, Error)]
#[error("{message}")]
struct TestError {
    message: String,
    status: u16,
}

impl CharacterProviderFailure for TestError {
    fn status_code(&self) -> Option<u16> {
        Some(self.status)
    }

    fn message(&self) -> String {
        self.message.clone()
    }
}

// the endpoint both character calls address
const CHARACTERS_URL: &str = "https://api.openai.com/v1/videos/characters";

struct Config {
    request_error: Option<&'static str>,
}

impl ErrorClasses for Config {
    type Error = TestError;

    fn get_error_class(
        &self,
        message: &str,
        status_code: u16,
        _headers: &ResponseHeaders,
    ) -> TestError {
        TestError {
            message: message.to_owned(),
            status: status_code,
        }
    }
}

impl CharacterConfig for Config {
    fn validate_environment(&self, environment: CharacterEnvironment<'_>) -> RequestHeaders {
        environment.headers
    }

    fn complete_url(
        &self,
        _model: &str,
        _api_base: Option<&str>,
        _litellm_params: &Params,
    ) -> Result<String, Self::Error> {
        Ok(CHARACTERS_URL.to_owned())
    }

    fn transform_create_request(
        &self,
        _name: &str,
        _video: &SuppliedMedia,
        _context: CharacterContext<'_>,
    ) -> Result<(String, Vec<MultipartPart>), Self::Error> {
        self.request_error.map_or_else(
            || Ok((CHARACTERS_URL.to_owned(), Vec::new())),
            |message| {
                Err(TestError {
                    message: message.to_owned(),
                    status: 400,
                })
            },
        )
    }

    fn transform_get_request(
        &self,
        _character_id: &str,
        _context: CharacterContext<'_>,
    ) -> Result<(String, Vec<(String, String)>), Self::Error> {
        Ok((format!("{CHARACTERS_URL}/char-1"), Vec::new()))
    }

    fn transform_create_response(
        &self,
        _reply: &UpstreamResponse,
        _logging: &mut LoggingState,
    ) -> Result<CharacterObject, Self::Error> {
        Ok(character())
    }

    fn transform_get_response(
        &self,
        _reply: &UpstreamResponse,
        _logging: &mut LoggingState,
    ) -> Result<CharacterObject, Self::Error> {
        Ok(character())
    }
}

fn character() -> CharacterObject {
    serde_json::from_str(CHARACTER_JSON).expect("the character fixture parses")
}

struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

type CallLogger = Box<dyn Fn(&Json) -> Result<(), PrePostError> + Send + Sync>;

struct LogEnv {
    redactor: Redactor,
    sink: Sink,
    now: DateTime<FixedOffset>,
    logger: CallLogger,
}

impl LogEnv {
    fn new() -> Self {
        Self {
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds"),
            sink: Sink,
            now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00")
                .expect("the clock parses"),
            logger: Box::new(|_details: &Json| Ok(())),
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
            logger_fn: Some(&*self.logger),
            now: self.now,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Sync,
    Asynchronous,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Sent {
    mode: Mode,
    timeout: Option<Duration>,
}

#[derive(Clone)]
struct Wire {
    sent: Arc<Mutex<Vec<Sent>>>,
}

impl Wire {
    fn new() -> Self {
        Self {
            sent: Arc::default(),
        }
    }

    fn record(
        &self,
        mode: Mode,
        request: CharacterRequest,
    ) -> Result<UpstreamResponse, RequestFailure> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Sent {
                mode,
                timeout: request.timeout,
            });
        Ok(UpstreamResponse {
            status: 200,
            headers: HeaderMap::new(),
            body: b"{}".to_vec(),
        })
    }
}

impl CharacterSyncClient for Wire {
    fn send_blocking(&self, request: CharacterRequest) -> Result<UpstreamResponse, RequestFailure> {
        self.record(Mode::Sync, request)
    }
}

impl CharacterAsyncClient for Wire {
    fn send(
        &self,
        request: CharacterRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, RequestFailure>> + Send {
        ready(self.record(Mode::Asynchronous, request))
    }
}

struct Clients {
    wire: Wire,
}

impl CharacterClients for Clients {
    type AsyncClient = Wire;
    type SyncClient = Wire;

    fn select_sync(
        &mut self,
        _params: &Params,
        supplied: Option<&Wire>,
    ) -> Result<Wire, ClientError> {
        Ok(supplied.map_or_else(|| self.wire.clone(), Clone::clone))
    }

    fn select_async(
        &mut self,
        _params: &Params,
        supplied: Option<&Wire>,
    ) -> Result<Wire, ClientError> {
        Ok(supplied.map_or_else(|| self.wire.clone(), Clone::clone))
    }
}

fn call_deps(redactor: &Redactor) -> CharacterCallDeps<'_> {
    CharacterCallDeps {
        redactor,
        request_timeout_seconds: 1234.0_f64,
    }
}

fn create_call() -> CreateCharacterCall {
    CreateCharacterCall {
        name: "Wren".to_owned(),
        video: video(),
        timeout: Some(DEFAULT_TIMEOUT_SECONDS),
        custom_llm_provider: DEFAULT_PROVIDER.to_owned(),
        extra_headers: Some(headers()),
        extra_query: None,
        extra_body: None,
        mock_response: None,
        litellm_call_id: Some(CALL_ID.to_owned()),
        extra: IndexMap::from([(
            "api_base".to_owned(),
            Json::Text("https://api.openai.com/v1".to_owned()),
        )]),
        is_async: false,
    }
}

fn get_call() -> GetCharacterCall {
    GetCharacterCall {
        character_id: "char-1".to_owned(),
        timeout: Some(DEFAULT_TIMEOUT_SECONDS),
        custom_llm_provider: DEFAULT_PROVIDER.to_owned(),
        extra_headers: None,
        extra_query: None,
        extra_body: None,
        mock_response: None,
        litellm_call_id: Some(CALL_ID.to_owned()),
        extra: IndexMap::from([(
            "api_base".to_owned(),
            Json::Text("https://api.openai.com/v1".to_owned()),
        )]),
        is_async: false,
    }
}

fn logging_state() -> LoggingState {
    let moment =
        DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").expect("the fixture is RFC 3339");
    LoggingState::new(
        LoggingInit {
            model: "character".to_owned(),
            call_type: "fixture_call_type".to_owned(),
            start_time: moment,
            litellm_call_id: CALL_ID.to_owned(),
            function_id: "fn-1".to_owned(),
            messages: None,
            stream: None,
            litellm_trace_id: Some("trace-1".to_owned()),
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds"),
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .expect("the logging fixture builds")
}

struct Fixture {
    config: Config,
    state: LoggingState,
    clients: Clients,
    wire: Wire,
    redactor: Redactor,
}

impl Fixture {
    fn new(request_error: Option<&'static str>) -> Self {
        let wire = Wire::new();
        Self {
            config: Config { request_error },
            state: logging_state(),
            clients: Clients { wire: wire.clone() },
            wire,
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds"),
        }
    }

    fn sent(&self) -> Vec<Sent> {
        self.wire
            .sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[test]
fn transport_adapter_create_answers_in_place() {
    let log = LogEnv::new();
    let env = log.env();
    let mut fixture = Fixture::new(None);
    let deps = call_deps(&fixture.redactor);
    let mut transport = CharacterTransport::with_generated_boundary(
        &fixture.config,
        &mut fixture.clients,
        &mut fixture.state,
        &env,
    );
    let call = create_call();

    let object =
        match video_create_character(&mut transport, deps, &call).expect("the transport answers") {
            CharacterOutcome::Done(object) => object,
            CharacterOutcome::Pending(_) => panic!("a blocking transport answers directly"),
        };
    assert_eq!(object, character());
    drop(transport);
    assert_eq!(
        fixture.sent(),
        vec![Sent {
            mode: Mode::Sync,
            timeout: Some(Duration::from_secs(600)),
        }]
    );
}

#[tokio::test]
async fn transport_adapter_create_awaits_the_async_client() {
    let log = LogEnv::new();
    let env = log.env();
    let mut fixture = Fixture::new(None);
    let deps = call_deps(&fixture.redactor);
    let mut transport = CharacterTransport::with_generated_boundary(
        &fixture.config,
        &mut fixture.clients,
        &mut fixture.state,
        &env,
    );
    let call = create_call();

    let object = avideo_create_character(&mut transport, deps, &call)
        .await
        .expect("the transport answers");

    assert_eq!(object, character());
    drop(transport);
    assert_eq!(
        fixture.sent(),
        vec![Sent {
            mode: Mode::Asynchronous,
            timeout: Some(Duration::from_secs(600)),
        }]
    );
}

#[tokio::test]
async fn transport_adapter_get_awaits_the_async_client() {
    let log = LogEnv::new();
    let env = log.env();
    let mut fixture = Fixture::new(None);
    let deps = call_deps(&fixture.redactor);
    let mut transport = CharacterTransport::with_generated_boundary(
        &fixture.config,
        &mut fixture.clients,
        &mut fixture.state,
        &env,
    );
    let call = get_call();

    let object = avideo_get_character(&mut transport, deps, &call)
        .await
        .expect("the transport answers");

    assert_eq!(object, character());
    drop(transport);
    assert_eq!(
        fixture.sent(),
        vec![Sent {
            mode: Mode::Asynchronous,
            timeout: None,
        }]
    );
}

#[test]
fn transport_adapter_get_answers_in_place() {
    let log = LogEnv::new();
    let env = log.env();
    let mut fixture = Fixture::new(None);
    let deps = call_deps(&fixture.redactor);
    let mut transport = CharacterTransport::with_generated_boundary(
        &fixture.config,
        &mut fixture.clients,
        &mut fixture.state,
        &env,
    );
    let call = get_call();

    let object =
        match video_get_character(&mut transport, deps, &call).expect("the transport answers") {
            CharacterOutcome::Done(object) => object,
            CharacterOutcome::Pending(_) => panic!("a blocking transport answers directly"),
        };
    assert_eq!(object, character());
    drop(transport);
    assert_eq!(
        fixture.sent(),
        vec![Sent {
            mode: Mode::Sync,
            timeout: None,
        }]
    );
}

#[test]
fn transport_adapter_maps_a_provider_fault() {
    let log = LogEnv::new();
    let env = log.env();
    let mut fixture = Fixture::new(Some("the video is unreadable"));
    let deps = call_deps(&fixture.redactor);
    let mut transport = CharacterTransport::with_generated_boundary(
        &fixture.config,
        &mut fixture.clients,
        &mut fixture.state,
        &env,
    );
    let call = create_call();

    let failure =
        video_create_character(&mut transport, deps, &call).expect_err("the transform fails");

    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!("expected a bad request, got {:?}", failure.class);
    };
    assert_eq!(details.status.0, 400);
    assert_eq!(details.provider.as_deref(), Some("openai"));
    assert_eq!(details.model.as_deref(), Some(""));
    assert!(details.message.contains("the video is unreadable"));
    drop(transport);
    assert!(fixture.sent().is_empty());
}

#[test]
fn transport_adapter_maps_a_client_fault_without_a_status() {
    let log = LogEnv::new();
    let env = log.env();
    let mut fixture = Fixture::new(None);
    let deps = call_deps(&fixture.redactor);
    let mut transport = CharacterTransport::with_generated_boundary(
        &fixture.config,
        &mut fixture.clients,
        &mut fixture.state,
        &env,
    );
    let mut call = create_call();
    call.extra.insert(
        "api_key".to_owned(),
        Json::Number(serde_json::Number::from(5_i64)),
    );

    let failure =
        video_create_character(&mut transport, deps, &call).expect_err("the key is not text");

    let ErrorClass::ApiConnection(context) = &failure.class else {
        panic!("expected an api connection, got {:?}", failure.class);
    };
    assert_eq!(context.provider.as_deref(), Some("openai"));
    assert!(context.message.contains("api_key"));
    drop(transport);
    assert!(fixture.sent().is_empty());
}

#[test]
fn transport_adapter_passes_the_resolved_fallback_timeout() {
    let log = LogEnv::new();
    let env = log.env();
    let mut fixture = Fixture::new(None);
    let deps = call_deps(&fixture.redactor);
    let mut transport = CharacterTransport::with_generated_boundary(
        &fixture.config,
        &mut fixture.clients,
        &mut fixture.state,
        &env,
    );
    let mut call = create_call();
    call.timeout = Some(0.0_f64);

    let object =
        match video_create_character(&mut transport, deps, &call).expect("the transport answers") {
            CharacterOutcome::Done(object) => object,
            CharacterOutcome::Pending(_) => panic!("a blocking transport answers directly"),
        };
    assert_eq!(object, character());
    drop(transport);
    assert_eq!(
        fixture.sent(),
        vec![Sent {
            mode: Mode::Sync,
            timeout: Some(Duration::from_millis(1_234_000)),
        }]
    );
}
