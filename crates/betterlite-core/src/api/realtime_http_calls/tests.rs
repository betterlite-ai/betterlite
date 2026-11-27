#![cfg(test)]

#[path = "tests/client_secret.rs"]
mod client_secret;

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error as StdError;
use std::fmt;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::api::realtime_http_calls::{RealtimeHttpCallSettings, load_settings};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_logging::logging_state::LoggingInit;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::{ResolutionContext, ResolutionError};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use http::header::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::runtime::{Builder, Runtime};

use super::RealtimeHttpCalls;
use super::call::{ClientSecretCall, RealtimeCallsCall, TranscriptionSessionCall};
use super::dispatch::{RealtimeHttpDispatch, RealtimeHttpRequest, SessionEndpoint};
use super::error::{RealtimeHttpCallsError, RecordError};
use super::provider::RealtimeHttpConfig;
use super::{DEFAULT_REALTIME_MODEL, DEFAULT_TRANSCRIPTION_MODEL};

const INIT: &str = r#"{"model":"gpt-4o-realtime-preview","call_type":"realtime","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const EMPTY_SETTINGS: &str = "litellm_settings: {}";
const GLOBALS: &str = "litellm_settings:\n  api_base: https://global.example/v1\n  api_key: sk-global\n  openai_key: sk-openai-global\n";
const OPENAI_MODEL: &str = "openai/gpt-4o-realtime-preview";

#[test]
fn timeout_none_and_zero_select_the_request_timeout() {
    let harness = Harness::new(EMPTY_SETTINGS, &[("REQUEST_TIMEOUT", "12.5")]);
    let kwargs = no_kwargs();
    harness
        .run_client_secret(client_secret_call(&kwargs, Some(OPENAI_MODEL)))
        .expect("the client secret");
    let mut zero = client_secret_call(&kwargs, Some(OPENAI_MODEL));
    zero.timeout = Some(0.0);
    harness.run_client_secret(zero).expect("the zero timeout");
    let mut transcription = transcription_call(&kwargs, None);
    transcription.timeout = Some(0.0);
    harness
        .run_transcription(transcription)
        .expect("the transcription session");
    let expected = Duration::try_from_secs_f64(12.5).expect("a duration");
    let seen = harness.seen();
    assert_eq!(seen.len(), 3);
    assert_eq!(session_timeout(&seen, 0), expected);
    assert_eq!(session_timeout(&seen, 1), expected);
    assert_eq!(session_timeout(&seen, 2), expected);
}

#[test]
fn timeout_a_positive_value_is_kept() {
    let harness = Harness::new(EMPTY_SETTINGS, &[("REQUEST_TIMEOUT", "12.5")]);
    let kwargs = no_kwargs();
    let mut call = client_secret_call(&kwargs, Some(OPENAI_MODEL));
    call.timeout = Some(2.5);
    harness.run_client_secret(call).expect("the client secret");
    let mut calls = calls_call(&kwargs, "ep-caller");
    calls.model = Some(OPENAI_MODEL);
    calls.timeout = Some(2.5);
    harness.run_calls(calls).expect("the realtime call");
    let expected = Duration::try_from_secs_f64(2.5).expect("a duration");
    let seen = harness.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(session_timeout(&seen, 0), expected);
    assert_eq!(
        calls_timeout(&seen, 1),
        Duration::try_from_secs_f64(2.5).expect("a duration")
    );
}

#[test]
fn transcription_defaults_to_the_bundled_whisper_model() {
    let harness = Harness::new(EMPTY_SETTINGS, &[]);
    let kwargs = no_kwargs();
    harness
        .run_transcription(transcription_call(&kwargs, None))
        .expect("the transcription session");
    let seen = harness.seen();
    let posting = only_session(&seen);
    assert_eq!(posting.endpoint, SessionEndpoint::TranscriptionSessions);
    assert_eq!(posting.provider, ProviderTag::OpenAi);
    assert_eq!(posting.model, DEFAULT_TRANSCRIPTION_MODEL);
    assert!(posting.request_data.is_empty());
    assert_eq!(posting.api_base, "https://api.openai.com");
    assert_eq!(posting.api_key, "");
    let logging = harness
        .logging
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(
        logging.optional_params,
        fields(r#"{"transcription_session":null}"#)
    );
}

#[test]
fn transcription_prefers_the_explicit_model_over_the_nested_one() {
    let harness = Harness::new(EMPTY_SETTINGS, &[]);
    let kwargs = no_kwargs();
    let session =
        json(r#"{"input_audio_transcription":{"model":"openai/other-model","language":"en"}}"#);
    let mut call = transcription_call(&kwargs, Some("openai/gpt-realtime-whisper"));
    call.transcription_session = Some(&session);
    harness
        .run_transcription(call)
        .expect("the transcription session");
    let seen = harness.seen();
    let posting = only_session(&seen);
    assert_eq!(posting.model, "gpt-realtime-whisper");
    assert_eq!(
        posting.request_data,
        fields(r#"{"input_audio_transcription":{"model":"gpt-realtime-whisper","language":"en"}}"#)
    );
}

#[test]
fn transcription_uses_the_nested_model_when_the_root_is_absent() {
    let harness = Harness::new(EMPTY_SETTINGS, &[]);
    let kwargs = no_kwargs();
    let session = json(
        r#"{"input_audio_transcription":{"model":"openai/gpt-realtime-whisper","language":"en"},
            "turn_detection":{"type":"server_vad"},"vendor_field":1,"null_field":null}"#,
    );
    let mut call = transcription_call(&kwargs, None);
    call.transcription_session = Some(&session);
    harness
        .run_transcription(call)
        .expect("the transcription session");
    let seen = harness.seen();
    let posting = only_session(&seen);
    assert_eq!(posting.model, "gpt-realtime-whisper");
    assert!(!posting.request_data.contains_key("model"));
    assert_eq!(
        posting.request_data,
        fields(
            r#"{"input_audio_transcription":{"model":"gpt-realtime-whisper","language":"en"},
                "turn_detection":{"type":"server_vad"},"vendor_field":1}"#
        )
    );
    let logging = harness
        .logging
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(
        logging.optional_params,
        fields(
            r#"{"transcription_session":{"input_audio_transcription":{"model":"openai/gpt-realtime-whisper","language":"en"},
                "turn_detection":{"type":"server_vad"},"vendor_field":1,"null_field":null}}"#
        )
    );
}

#[test]
fn transcription_does_not_create_a_missing_nested_transcription() {
    let harness = Harness::new(EMPTY_SETTINGS, &[]);
    let kwargs = no_kwargs();
    let session = json(r#"{"input_audio_format":"pcm16"}"#);
    let mut call = transcription_call(&kwargs, None);
    call.transcription_session = Some(&session);
    harness
        .run_transcription(call)
        .expect("the transcription session");
    let seen = harness.seen();
    let posting = only_session(&seen);
    assert_eq!(posting.model, DEFAULT_TRANSCRIPTION_MODEL);
    assert_eq!(
        posting.request_data,
        fields(r#"{"input_audio_format":"pcm16"}"#)
    );
}

#[test]
fn transcription_adds_the_model_to_an_empty_nested_transcription() {
    let harness = Harness::new(EMPTY_SETTINGS, &[]);
    let kwargs = no_kwargs();
    let session = json(r#"{"input_audio_transcription":{}}"#);
    let mut call = transcription_call(&kwargs, None);
    call.transcription_session = Some(&session);
    harness
        .run_transcription(call)
        .expect("the transcription session");
    let seen = harness.seen();
    let posting = only_session(&seen);
    assert_eq!(
        posting.request_data,
        fields(r#"{"input_audio_transcription":{"model":"gpt-realtime-whisper"}}"#)
    );
}

#[test]
fn transcription_rejects_a_root_model_inside_the_session_before_any_request() {
    let harness = Harness::new(EMPTY_SETTINGS, &[]);
    let kwargs = no_kwargs();
    let session = json(r#"{"model":"openai/x"}"#);
    let mut call = transcription_call(&kwargs, Some("openai/y"));
    call.transcription_session = Some(&session);
    let error = harness
        .run_transcription(call)
        .expect_err("the root model is duplicated");
    let RealtimeHttpCallsError::Record(RecordError::ConflictingModel) = error else {
        panic!("the duplicate model is refused");
    };
    assert!(harness.seen().is_empty());
}

#[test]
fn transcription_sends_azure_credentials_and_ignores_a_custom_llm_provider_kwarg() {
    let harness = Harness::new(EMPTY_SETTINGS, &[]);
    let mut kwargs = no_kwargs();
    kwargs.insert(
        String::from("custom_llm_provider"),
        Json::Text(String::from("openai")),
    );
    let key = secret("azure-key");
    let mut call = transcription_call(&kwargs, Some("azure/deployment-a"));
    call.api_base = Some("https://azure.example");
    call.api_key = Some(&key);
    call.api_version = Some("2024-10-01");
    harness
        .run_transcription(call)
        .expect("the transcription session");
    let seen = harness.seen();
    let posting = only_session(&seen);
    assert_eq!(posting.provider, ProviderTag::Azure);
    assert_eq!(posting.model, "deployment-a");
    assert_eq!(posting.api_base, "https://azure.example");
    assert_eq!(posting.api_key, "azure-key");
    assert_eq!(posting.api_version.as_deref(), Some("2024-10-01"));
    let logging = harness
        .logging
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(logging.model, "deployment-a");
    assert_eq!(logging.custom_llm_provider.as_deref(), Some("azure"));
    assert_eq!(
        logging.litellm_params,
        fields(r#"{"api_base":"https://azure.example"}"#)
    );
}

#[test]
fn transcription_logs_the_raw_transcription_session() {
    let harness = Harness::new(EMPTY_SETTINGS, &[]);
    let kwargs = no_kwargs();
    let session = json(r#"{"input_audio_transcription":{"model":"openai/gpt-realtime-whisper"}}"#);
    let mut call = transcription_call(&kwargs, None);
    call.transcription_session = Some(&session);
    harness
        .run_transcription(call)
        .expect("the transcription session");
    let logging = harness
        .logging
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(
        logging.optional_params,
        fields(
            r#"{"transcription_session":{"input_audio_transcription":{"model":"openai/gpt-realtime-whisper"}}}"#
        )
    );
    assert_eq!(logging.model, "gpt-realtime-whisper");
}

#[test]
fn calls_rejects_the_default_model_without_a_request() {
    let harness = Harness::new(EMPTY_SETTINGS, &[]);
    let kwargs = no_kwargs();
    let error = harness
        .run_calls(calls_call(&kwargs, "ep-caller"))
        .expect_err("the default model resolves no provider");
    let RealtimeHttpCallsError::Resolution(ResolutionError::MissingProvider { model }) = error
    else {
        panic!("the default model is unresolvable");
    };
    assert_eq!(model, DEFAULT_REALTIME_MODEL);
    assert!(harness.seen().is_empty());
}

#[test]
fn calls_keeps_the_callers_ephemeral_key() {
    let harness = Harness::new(
        "litellm_settings:\n  api_key: sk-global\n",
        &[("OPENAI_API_KEY", "sk-env")],
    );
    let kwargs = no_kwargs();
    let mut call = calls_call(&kwargs, "ep-caller");
    call.model = Some(OPENAI_MODEL);
    call.sdp_body = b"v=0\r\n".as_slice();
    harness.run_calls(call).expect("the realtime call");
    let seen = harness.seen();
    let posting = only_calls(&seen);
    assert_eq!(posting.ephemeral_key, "ep-caller");
    assert_eq!(posting.model, "gpt-4o-realtime-preview");
    assert_eq!(posting.sdp, b"v=0\r\n".as_slice());
    assert_eq!(posting.provider, ProviderTag::OpenAi);
}

#[test]
fn calls_rewrites_an_existing_session_model() {
    let harness = Harness::new(EMPTY_SETTINGS, &[]);
    let kwargs = no_kwargs();
    let session = json(r#"{"type":"transcription","model":null,"voice":"alloy"}"#);
    let mut call = calls_call(&kwargs, "ep-caller");
    call.model = Some(OPENAI_MODEL);
    call.session = Some(&session);
    harness.run_calls(call).expect("the realtime call");
    let seen = harness.seen();
    let posting = only_calls(&seen);
    let expected =
        fields(r#"{"type":"transcription","model":"gpt-4o-realtime-preview","voice":"alloy"}"#);
    assert_eq!(posting.session.as_ref(), Some(&expected));
    let logging = harness
        .logging
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(
        logging.optional_params,
        IndexMap::from([
            (String::from("realtime_calls"), Json::Bool(true)),
            (String::from("session"), Json::Object(expected)),
        ])
    );
}

#[test]
fn calls_leaves_an_absent_session_model_absent() {
    let harness = Harness::new(EMPTY_SETTINGS, &[]);
    let kwargs = no_kwargs();
    let session = json(r#"{"type":"transcription"}"#);
    let mut call = calls_call(&kwargs, "ep-caller");
    call.model = Some(OPENAI_MODEL);
    call.session = Some(&session);
    harness.run_calls(call).expect("the realtime call");
    let seen = harness.seen();
    let posting = only_calls(&seen);
    assert_eq!(posting.session, Some(fields(r#"{"type":"transcription"}"#)));
}

#[test]
fn calls_without_a_session_logs_null() {
    let harness = Harness::new(EMPTY_SETTINGS, &[]);
    let kwargs = no_kwargs();
    let mut call = calls_call(&kwargs, "ep-caller");
    call.model = Some(OPENAI_MODEL);
    harness.run_calls(call).expect("the realtime call");
    let seen = harness.seen();
    let posting = only_calls(&seen);
    assert_eq!(posting.session, None);
    let logging = harness
        .logging
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(
        logging.optional_params,
        fields(r#"{"realtime_calls":true,"session":null}"#)
    );
}

#[test]
fn calls_forwards_the_sdp_and_returns_the_raw_response() {
    let harness = Harness::new(EMPTY_SETTINGS, &[]);
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/sdp"));
    harness.recorder.reply(200, b"v=0\r\na=answer\r\n", headers);
    let kwargs = no_kwargs();
    let mut call = calls_call(&kwargs, "ep-caller");
    call.model = Some(OPENAI_MODEL);
    call.sdp_body = b"v=0\r\no=offer\r\n".as_slice();
    let response = harness.run_calls(call).expect("the realtime call");
    let seen = harness.seen();
    let posting = only_calls(&seen);
    assert_eq!(posting.sdp, b"v=0\r\no=offer\r\n".as_slice());
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"v=0\r\na=answer\r\n".to_vec());
    assert_eq!(
        response
            .headers
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/sdp")
    );
}

struct Harness {
    settings: RealtimeHttpCallSettings,
    environment: BTreeMap<String, String>,
    catalog: Arc<ModelCatalog>,
    known: BTreeSet<String>,
    custom: BTreeSet<String>,
    recorder: Recorder,
    logging: Mutex<LoggingState>,
}

impl Harness {
    fn new(source: &str, environment: &[(&str, &str)]) -> Self {
        let values: BTreeMap<String, String> = environment
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        let settings = load_settings(source, values.clone()).expect("the settings load");
        let mut bundled = BundledCatalog::default();
        let catalog = bundled.get();
        Self {
            settings,
            environment: values,
            catalog,
            known: BTreeSet::new(),
            custom: BTreeSet::new(),
            recorder: Recorder::default(),
            logging: Mutex::new(logging_state()),
        }
    }

    fn resolution(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: self.catalog.as_ref(),
            known_model_names: &self.known,
            custom_providers: &self.custom,
            environment: &self.environment,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }

    fn calls(&self) -> RealtimeHttpCalls<'_, Recorder> {
        RealtimeHttpCalls {
            dispatch: &self.recorder,
            settings: &self.settings,
            resolution: self.resolution(),
        }
    }

    fn run_client_secret(
        &self,
        call: ClientSecretCall<'_>,
    ) -> Result<UpstreamResponse, RealtimeHttpCallsError<FakeFailure>> {
        let mut logging = self.logging.lock().unwrap_or_else(PoisonError::into_inner);
        runtime().block_on(
            self.calls()
                .acreate_realtime_client_secret(call, &mut logging),
        )
    }

    fn run_transcription(
        &self,
        call: TranscriptionSessionCall<'_>,
    ) -> Result<UpstreamResponse, RealtimeHttpCallsError<FakeFailure>> {
        let mut logging = self.logging.lock().unwrap_or_else(PoisonError::into_inner);
        runtime().block_on(
            self.calls()
                .acreate_realtime_transcription_session(call, &mut logging),
        )
    }

    fn run_calls(
        &self,
        call: RealtimeCallsCall<'_>,
    ) -> Result<UpstreamResponse, RealtimeHttpCallsError<FakeFailure>> {
        let mut logging = self.logging.lock().unwrap_or_else(PoisonError::into_inner);
        runtime().block_on(self.calls().arealtime_calls(call, &mut logging))
    }

    fn seen(&self) -> Vec<Seen> {
        self.recorder.seen()
    }
}

fn logging_state() -> LoggingState {
    let mut context = CorrelationContext::default();
    LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).expect("the logging init"),
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor"),
        || String::from("trace"),
        Duration::default,
    )
    .expect("the logging state")
}

fn runtime() -> Runtime {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the test runtime")
}

fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("a JSON value")
}

fn fields(source: &str) -> IndexMap<String, Json> {
    let Json::Object(fields) = json(source) else {
        panic!("a JSON object");
    };
    fields
}

fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn no_kwargs() -> IndexMap<String, Json> {
    IndexMap::new()
}

fn client_secret_call<'call>(
    kwargs: &'call IndexMap<String, Json>,
    model: Option<&'call str>,
) -> ClientSecretCall<'call> {
    ClientSecretCall {
        model,
        session: None,
        expires_after: None,
        timeout: None,
        api_base: None,
        api_key: None,
        api_version: None,
        extra_headers: None,
        client: None,
        kwargs,
    }
}

fn transcription_call<'call>(
    kwargs: &'call IndexMap<String, Json>,
    model: Option<&'call str>,
) -> TranscriptionSessionCall<'call> {
    TranscriptionSessionCall {
        model,
        transcription_session: None,
        timeout: None,
        api_base: None,
        api_key: None,
        api_version: None,
        extra_headers: None,
        client: None,
        kwargs,
    }
}

fn calls_call<'call>(
    kwargs: &'call IndexMap<String, Json>,
    ephemeral_key: &'call str,
) -> RealtimeCallsCall<'call> {
    RealtimeCallsCall {
        openai_ephemeral_key: ephemeral_key,
        sdp_body: b"",
        model: None,
        session: None,
        timeout: None,
        api_base: None,
        api_key: None,
        api_version: None,
        extra_headers: None,
        client: None,
        kwargs,
    }
}

#[derive(Debug, Clone)]
enum Seen {
    Session(SeenSession),
    Calls(SeenCalls),
}

#[derive(Debug, Clone)]
struct SeenSession {
    endpoint: SessionEndpoint,
    provider: ProviderTag,
    api_base: String,
    api_key: String,
    request_data: IndexMap<String, Json>,
    model: String,
    extra_headers: Option<BTreeMap<String, String>>,
    api_version: Option<String>,
    timeout: Duration,
    client_kind: Option<ClientKind>,
}

#[derive(Debug, Clone)]
struct SeenCalls {
    provider: ProviderTag,
    api_base: String,
    ephemeral_key: String,
    sdp: Vec<u8>,
    model: String,
    session: Option<IndexMap<String, Json>>,
    api_version: Option<String>,
    timeout: Duration,
    client_kind: Option<ClientKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderTag {
    OpenAi,
    Azure,
    None,
}

fn tag(config: Option<&RealtimeHttpConfig>) -> ProviderTag {
    match config {
        Some(RealtimeHttpConfig::OpenAi(_)) => ProviderTag::OpenAi,
        Some(RealtimeHttpConfig::Azure(_)) => ProviderTag::Azure,
        None => ProviderTag::None,
    }
}

fn record(request: RealtimeHttpRequest<'_>) -> Seen {
    match request {
        RealtimeHttpRequest::Session(session) => Seen::Session(SeenSession {
            endpoint: session.endpoint,
            provider: tag(session.provider),
            api_base: session.api_base.to_owned(),
            api_key: session.api_key.to_owned(),
            request_data: session.request_data.clone(),
            model: session.model.to_owned(),
            extra_headers: session.extra_headers.cloned(),
            api_version: session.api_version.map(str::to_owned),
            timeout: session.timeout,
            client_kind: session.client.map(|client| client.kind),
        }),
        RealtimeHttpRequest::Calls(calls) => Seen::Calls(SeenCalls {
            provider: tag(calls.provider),
            api_base: calls.api_base.to_owned(),
            ephemeral_key: calls.ephemeral_key.to_owned(),
            sdp: calls.sdp.to_vec(),
            model: calls.model.to_owned(),
            session: calls.session.cloned(),
            api_version: calls.api_version.map(str::to_owned),
            timeout: calls.timeout,
            client_kind: calls.client.map(|client| client.kind),
        }),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FakeFailure {
    message: &'static str,
}

impl fmt::Display for FakeFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message)
    }
}

impl StdError for FakeFailure {}

#[derive(Default)]
struct Recorder {
    seen: Mutex<Vec<Seen>>,
    reply: Mutex<Option<UpstreamResponse>>,
    failure: Mutex<Option<FakeFailure>>,
}

impl Recorder {
    fn reply(&self, status: u16, body: &[u8], headers: HeaderMap) {
        let answer = UpstreamResponse {
            status,
            headers,
            body: body.to_vec(),
        };
        *self.reply.lock().unwrap_or_else(PoisonError::into_inner) = Some(answer);
    }

    fn fail_once(&self, message: &'static str) {
        *self.failure.lock().unwrap_or_else(PoisonError::into_inner) =
            Some(FakeFailure { message });
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl RealtimeHttpDispatch for Recorder {
    type Error = FakeFailure;

    fn exchange(
        &self,
        request: RealtimeHttpRequest<'_>,
        _logging: &mut LoggingState,
    ) -> impl Future<Output = Result<UpstreamResponse, FakeFailure>> + Send {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record(request));
        let answer = self
            .reply
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let failure = self
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        ready(match failure {
            Some(failure) => Err(failure),
            None => Ok(answer.unwrap_or_else(|| UpstreamResponse {
                status: 200,
                headers: HeaderMap::new(),
                body: b"{}".to_vec(),
            })),
        })
    }
}

fn only_session(seen: &[Seen]) -> &SeenSession {
    assert_eq!(seen.len(), 1, "one session POST");
    let Seen::Session(posting) = &seen[0] else {
        panic!("the request is a session POST");
    };
    posting
}

fn only_calls(seen: &[Seen]) -> &SeenCalls {
    assert_eq!(seen.len(), 1, "one calls POST");
    let Seen::Calls(posting) = &seen[0] else {
        panic!("the request is a calls POST");
    };
    posting
}

fn session_timeout(seen: &[Seen], index: usize) -> Duration {
    let Seen::Session(posting) = &seen[index] else {
        panic!("the request is a session POST");
    };
    posting.timeout
}

fn calls_timeout(seen: &[Seen], index: usize) -> Duration {
    let Seen::Calls(posting) = &seen[index] else {
        panic!("the request is a calls POST");
    };
    posting.timeout
}
