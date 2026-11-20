use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_config::types::ProxyTrust;
use betterlite_http::interactions::base::clients::{HttpClients, async_client};
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
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::header::HeaderMap;
use reqwest::Client;
use reqwest::blocking::Client as BlockingClient;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::api::bfl::edit::media::SuppliedMedia;
use crate::api::video_character_transport::clients::{
    CharacterAsyncClient, CharacterClients, CharacterRequest, CharacterSyncClient, SuppliedClients,
};
use crate::api::video_character_transport::config::{
    CharacterConfig, CharacterContext, CharacterEnvironment,
};
use crate::api::video_character_transport::error::CharacterFault;
use crate::api::video_character_transport::service::{
    CharacterCall, CharacterCreateCall, CharacterGetCall, CharacterHandled, CharacterTransport,
};
use crate::api::video_records::CharacterObject;

pub(crate) const CHARACTER_BODY: &str =
    r#"{"id":"char-1","object":"character","created_at":1700000000,"name":"Wren"}"#;

#[derive(Clone, Default)]
pub(crate) struct Events(Arc<Mutex<Vec<String>>>);

impl Events {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn push(&self, name: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(name.to_owned());
    }

    pub(crate) fn recorded(&self) -> Vec<String> {
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

// the logging environment a pre-call needs, with a logger fn that records the call in the shared events
pub(crate) struct LogEnv {
    redactor: Redactor,
    sink: Sink,
    now: DateTime<FixedOffset>,
    logger: Box<dyn Fn(&Json) -> Result<(), PrePostError> + Sync>,
}

impl LogEnv {
    pub(crate) fn for_events(events: &Events) -> Self {
        let recorded = events.clone();
        let logger = Box::new(move |_details: &Json| -> Result<(), PrePostError> {
            recorded.push("log");
            Ok(())
        });
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
        let now =
            DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").expect("the clock parses");
        Self {
            redactor,
            sink: Sink,
            now,
            logger,
        }
    }

    pub(crate) fn env(&self) -> PrePostEnv<'_> {
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

pub(crate) fn logging_state(call_type: &str) -> LoggingState {
    let init: LoggingInit = serde_json::from_str(&format!(
        r#"{{"model":"video","call_type":"{call_type}","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}}"#
    ))
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

// the provider error class the recording config builds
#[derive(Debug, Error)]
#[error("{message}")]
pub(crate) struct TestError {
    pub(crate) message: String,
    pub(crate) status_code: u16,
    pub(crate) headers: ResponseHeaders,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Seen {
    Environment {
        api_key: Option<String>,
        model: String,
        headers: Vec<(String, String)>,
    },
    Url {
        model: String,
        api_base: Option<String>,
    },
    CreateRequest {
        name: String,
        video: Vec<u8>,
        api_base: String,
        headers: Vec<(String, String)>,
    },
    GetRequest {
        character_id: String,
        api_base: String,
        headers: Vec<(String, String)>,
    },
    Reply {
        status: u16,
        body: Vec<u8>,
    },
}

pub(crate) struct RecordingConfig {
    pub(crate) events: Events,
    pub(crate) create_url: String,
    pub(crate) get_url: String,
    pub(crate) query: Vec<(String, String)>,
    pub(crate) environment: RequestHeaders,
    pub(crate) files: Option<Vec<MultipartPart>>,
    pub(crate) object: CharacterObject,
    pub(crate) url_error: Option<String>,
    pub(crate) request_error: Option<String>,
    pub(crate) reply_error: Option<String>,
    pub(crate) seen: Mutex<Vec<Seen>>,
    pub(crate) errors: Mutex<Vec<(String, u16, ResponseHeaders)>>,
}

pub(crate) fn recording(events: &Events) -> RecordingConfig {
    let base = "https://api.example/v1";
    RecordingConfig {
        events: events.clone(),
        create_url: format!("{base}/characters"),
        get_url: format!("{base}/characters/char-1"),
        query: Vec::new(),
        environment: RequestHeaders::new(),
        files: None,
        object: character_object("Wren"),
        url_error: None,
        request_error: None,
        reply_error: None,
        seen: Mutex::default(),
        errors: Mutex::default(),
    }
}

impl RecordingConfig {
    pub(crate) fn seen(&self) -> Vec<Seen> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn errors(&self) -> Vec<(String, u16, ResponseHeaders)> {
        self.errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, seen: Seen) {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(seen);
    }

    fn transformed(
        &self,
        event: &str,
        reply: &UpstreamResponse,
    ) -> Result<CharacterObject, TestError> {
        self.events.push(event);
        self.record(Seen::Reply {
            status: reply.status,
            body: reply.body.clone(),
        });
        self.reply_error.as_deref().map_or_else(
            || Ok(self.object.clone()),
            |message| Err(test_error(message)),
        )
    }
}

impl ErrorClasses for RecordingConfig {
    type Error = TestError;

    fn get_error_class(
        &self,
        message: &str,
        status_code: u16,
        headers: &ResponseHeaders,
    ) -> TestError {
        self.errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((message.to_owned(), status_code, headers.clone()));
        TestError {
            message: message.to_owned(),
            status_code,
            headers: headers.clone(),
        }
    }
}

impl CharacterConfig for RecordingConfig {
    fn validate_environment(&self, environment: CharacterEnvironment<'_>) -> RequestHeaders {
        self.events.push("validate");
        self.record(Seen::Environment {
            api_key: environment
                .api_key
                .map(|key| key.expose_secret().to_owned()),
            model: environment.model.to_owned(),
            headers: header_pairs(&environment.headers),
        });
        let mut headers = environment.headers;
        headers.extend(self.environment.clone());
        headers
    }

    fn complete_url(
        &self,
        model: &str,
        api_base: Option<&str>,
        _litellm_params: &Params,
    ) -> Result<String, Self::Error> {
        self.events.push("url");
        self.record(Seen::Url {
            model: model.to_owned(),
            api_base: api_base.map(str::to_owned),
        });
        self.url_error.as_deref().map_or_else(
            || Ok(self.create_url.clone()),
            |message| Err(test_error(message)),
        )
    }

    fn transform_create_request(
        &self,
        name: &str,
        video: &SuppliedMedia,
        context: CharacterContext<'_>,
    ) -> Result<(String, Vec<MultipartPart>), Self::Error> {
        self.events.push("transform-create");
        self.record(Seen::CreateRequest {
            name: name.to_owned(),
            video: video_bytes(video),
            api_base: context.api_base.to_owned(),
            headers: header_pairs(context.headers),
        });
        if let Some(message) = &self.request_error {
            return Err(test_error(message));
        }
        let files = self
            .files
            .as_ref()
            .map_or_else(|| default_parts(name, video), Clone::clone);
        Ok((self.create_url.clone(), files))
    }

    fn transform_get_request(
        &self,
        character_id: &str,
        context: CharacterContext<'_>,
    ) -> Result<(String, Vec<(String, String)>), Self::Error> {
        self.events.push("transform-get");
        self.record(Seen::GetRequest {
            character_id: character_id.to_owned(),
            api_base: context.api_base.to_owned(),
            headers: header_pairs(context.headers),
        });
        self.request_error.as_deref().map_or_else(
            || Ok((self.get_url.clone(), self.query.clone())),
            |message| Err(test_error(message)),
        )
    }

    fn transform_create_response(
        &self,
        reply: &UpstreamResponse,
        _logging: &mut LoggingState,
    ) -> Result<CharacterObject, Self::Error> {
        self.transformed("transform-create-response", reply)
    }

    fn transform_get_response(
        &self,
        reply: &UpstreamResponse,
        _logging: &mut LoggingState,
    ) -> Result<CharacterObject, Self::Error> {
        self.transformed("transform-get-response", reply)
    }
}

fn test_error(message: &str) -> TestError {
    TestError {
        message: message.to_owned(),
        status_code: 0,
        headers: ResponseHeaders::new(),
    }
}

fn header_pairs(headers: &RequestHeaders) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect()
}

fn video_bytes(video: &SuppliedMedia) -> Vec<u8> {
    match video {
        SuppliedMedia::Bytes(bytes) => bytes.clone(),
        SuppliedMedia::Text(_) | SuppliedMedia::File(_) | SuppliedMedia::List(_) => Vec::new(),
    }
}

// the parts a character creation sends when the fixture leaves its own out
fn default_parts(name: &str, video: &SuppliedMedia) -> Vec<MultipartPart> {
    vec![
        MultipartPart {
            name: "name".to_owned(),
            file_name: None,
            content_type: None,
            data: name.as_bytes().to_vec(),
        },
        MultipartPart {
            name: "video".to_owned(),
            file_name: Some("clip.mp4".to_owned()),
            content_type: Some("video/mp4".to_owned()),
            data: video_bytes(video),
        },
    ]
}

// the wire a character call sends through: it records the request and answers or fails with the fixture's choice
#[derive(Clone)]
pub(crate) struct FakeWire {
    events: Events,
    pub(crate) status: u16,
    pub(crate) headers: HeaderMap,
    pub(crate) body: Vec<u8>,
    requests: Arc<Mutex<Vec<CharacterRequest>>>,
    failure: Arc<Mutex<Option<RequestFailure>>>,
}

impl FakeWire {
    pub(crate) fn new(events: &Events, status: u16, body: &str) -> Self {
        Self {
            events: events.clone(),
            status,
            headers: HeaderMap::new(),
            body: body.as_bytes().to_vec(),
            requests: Arc::default(),
            failure: Arc::default(),
        }
    }

    pub(crate) fn failing(events: &Events, failure: RequestFailure) -> Self {
        let wire = Self::new(events, 200, "{}");
        *wire.failure.lock().unwrap_or_else(PoisonError::into_inner) = Some(failure);
        wire
    }

    pub(crate) fn requests(&self) -> MutexGuard<'_, Vec<CharacterRequest>> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn record(&self, request: CharacterRequest) -> Result<UpstreamResponse, RequestFailure> {
        self.events.push("send");
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
                Ok(UpstreamResponse {
                    status: self.status,
                    headers: self.headers.clone(),
                    body: self.body.clone(),
                })
            },
            Err,
        )
    }
}

impl CharacterSyncClient for FakeWire {
    fn send_blocking(&self, request: CharacterRequest) -> Result<UpstreamResponse, RequestFailure> {
        self.record(request)
    }
}

impl CharacterAsyncClient for FakeWire {
    fn send(
        &self,
        request: CharacterRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, RequestFailure>> + Send {
        ready(self.record(request))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Sync,
    Asynchronous,
}

// the client source the handlers select through: it records what it was asked for, then hands out the supplied wire or its own
pub(crate) struct FakeClients {
    events: Events,
    wire: FakeWire,
    failure: Option<ClientError>,
    selections: Mutex<Vec<(Mode, Option<JsonValue>)>>,
}

impl FakeClients {
    pub(crate) fn from_wire(events: &Events, wire: FakeWire) -> Self {
        Self {
            events: events.clone(),
            wire,
            failure: None,
            selections: Mutex::default(),
        }
    }

    pub(crate) fn failing(events: &Events, wire: FakeWire, failure: ClientError) -> Self {
        let mut clients = Self::from_wire(events, wire);
        clients.failure = Some(failure);
        clients
    }

    pub(crate) fn selections(&self) -> Vec<(Mode, Option<JsonValue>)> {
        self.selections
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn selected(
        &mut self,
        mode: Mode,
        params: &Params,
        supplied: Option<&FakeWire>,
    ) -> Result<FakeWire, ClientError> {
        self.events.push(match mode {
            Mode::Sync => "select-sync",
            Mode::Asynchronous => "select-async",
        });
        self.selections
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((mode, params.get("ssl_verify").cloned()));
        self.failure.take().map_or_else(
            || Ok(supplied.map_or_else(|| self.wire.clone(), Clone::clone)),
            Err,
        )
    }
}

impl CharacterClients for FakeClients {
    type AsyncClient = FakeWire;
    type SyncClient = FakeWire;

    fn select_sync(
        &mut self,
        params: &Params,
        supplied: Option<&FakeWire>,
    ) -> Result<FakeWire, ClientError> {
        self.selected(Mode::Sync, params, supplied)
    }

    fn select_async(
        &mut self,
        params: &Params,
        supplied: Option<&FakeWire>,
    ) -> Result<FakeWire, ClientError> {
        self.selected(Mode::Asynchronous, params, supplied)
    }
}

// one test's config, logging fixture, clients and wire, so a case only builds its call
pub(crate) struct Fixture {
    pub(crate) config: RecordingConfig,
    pub(crate) log: LogEnv,
    pub(crate) state: LoggingState,
    pub(crate) clients: FakeClients,
    pub(crate) wire: FakeWire,
    pub(crate) events: Events,
}

impl Fixture {
    pub(crate) fn new() -> Self {
        let events = Events::new();
        let wire = FakeWire::new(&events, 200, CHARACTER_BODY);
        let clients = FakeClients::from_wire(&events, wire.clone());
        Self {
            config: recording(&events),
            log: LogEnv::for_events(&events),
            state: logging_state("video_create_character"),
            clients,
            wire,
            events,
        }
    }

    // a case that needs the wire to answer differently swaps it in, and the client source follows it
    pub(crate) fn use_wire(&mut self, wire: FakeWire) {
        self.clients = FakeClients::from_wire(&self.events, wire.clone());
        self.wire = wire;
    }

    pub(crate) fn fail_selection(&mut self, failure: ClientError) {
        self.clients = FakeClients::failing(&self.events, self.wire.clone(), failure);
    }

    pub(crate) fn sync_create(
        &mut self,
        call: &CharacterCreateCall<'_>,
        supplied: Option<&FakeWire>,
    ) -> Result<CharacterObject, CharacterFault<RecordingConfig>> {
        self.sync(CharacterCall::Create(call), supplied)
    }

    pub(crate) fn sync_get(
        &mut self,
        call: &CharacterGetCall<'_>,
        supplied: Option<&FakeWire>,
    ) -> Result<CharacterObject, CharacterFault<RecordingConfig>> {
        self.sync(CharacterCall::Get(call), supplied)
    }

    pub(crate) async fn async_create(
        &mut self,
        call: &CharacterCreateCall<'_>,
        supplied: Option<&FakeWire>,
    ) -> Result<CharacterObject, CharacterFault<RecordingConfig>> {
        self.async_flow(CharacterCall::Create(call), supplied).await
    }

    pub(crate) async fn async_get(
        &mut self,
        call: &CharacterGetCall<'_>,
        supplied: Option<&FakeWire>,
    ) -> Result<CharacterObject, CharacterFault<RecordingConfig>> {
        self.async_flow(CharacterCall::Get(call), supplied).await
    }

    fn sync(
        &mut self,
        call: CharacterCall<'_>,
        supplied: Option<&FakeWire>,
    ) -> Result<CharacterObject, CharacterFault<RecordingConfig>> {
        let env = self.log.env();
        let mut transport = CharacterTransport::with_generated_boundary(
            &self.config,
            &mut self.clients,
            &mut self.state,
            &env,
        )
        .supplying(SuppliedClients {
            sync: supplied,
            asynchronous: None,
        });
        match transport.video_character_handler(call)? {
            CharacterHandled::Done(object) => Ok(object),
            CharacterHandled::Pending(_) => {
                panic!("a sync call without the async flag answers in place")
            }
        }
    }

    async fn async_flow(
        &mut self,
        call: CharacterCall<'_>,
        supplied: Option<&FakeWire>,
    ) -> Result<CharacterObject, CharacterFault<RecordingConfig>> {
        let env = self.log.env();
        let mut transport = CharacterTransport::with_generated_boundary(
            &self.config,
            &mut self.clients,
            &mut self.state,
            &env,
        );
        transport.supplied = SuppliedClients {
            sync: None,
            asynchronous: supplied,
        };
        let reply = transport.async_video_character_handler(call);
        reply.await
    }
}

pub(crate) fn create_call<'call>(
    name: &'call str,
    video: &'call SuppliedMedia,
    params: &'call Params,
) -> CharacterCreateCall<'call> {
    CharacterCreateCall {
        name,
        video,
        litellm_params: params,
        extra_headers: None,
        timeout: None,
        api_key: None,
        is_async: false,
    }
}

pub(crate) fn get_call<'call>(
    character_id: &'call str,
    params: &'call Params,
) -> CharacterGetCall<'call> {
    CharacterGetCall {
        character_id,
        litellm_params: params,
        extra_headers: None,
        timeout: None,
        api_key: None,
        is_async: false,
    }
}

pub(crate) fn params(pairs: &[(&str, &str)]) -> Params {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), JsonValue::Text((*value).to_owned())))
        .collect()
}

pub(crate) fn extra_headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

pub(crate) fn video() -> SuppliedMedia {
    SuppliedMedia::Bytes(b"video-bytes".to_vec())
}

pub(crate) fn character_object(name: &str) -> CharacterObject {
    let body = format!(
        r#"{{"id":"char-1","object":"character","created_at":1700000000,"name":"{name}"}}"#
    );
    serde_json::from_str(&body).expect("the character fixture is valid")
}

// the real client source the selection tests hand the handlers, built the way the gateway builds it
pub(crate) fn real_clients() -> HttpClients {
    let proxy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
        .expect("the proxy policy resolves");
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

pub(crate) fn blocking_client() -> BlockingClient {
    let mut clients = real_clients();
    clients
        .sync_client(&Params::new(), None)
        .expect("the blocking client builds")
}

pub(crate) fn async_http_client() -> Client {
    let mut clients = real_clients();
    async_client(&mut clients, &Params::new(), None).expect("the async client builds")
}
