#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::decisions::Json;
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};

use crate::api::api_boundaries::ApiLimits;
use crate::api::realtime_contracts::config::{BaseRealtimeConfig, Environment};
use crate::api::realtime_contracts::error::{HeaderMap, WebSocketError};
use crate::api::realtime_contracts::events::{
    AllDeltaType, RealtimeResponseTransformInput, RealtimeResponseTypedDict,
};
use crate::api::realtime_contracts::transport::{RealtimeBackend, RealtimeMessage};
use crate::api::vertex_chirp_protocol::events::ChirpBackendEvent;

use super::config::VertexChirpRealtimeConfig;
use super::decode::decode_chirp_backend_event;
use super::error::ChirpAdapterError;
use super::factory::ChirpBackendFactory;
use super::ids::{ChirpIds, UuidChirpIds};
use super::settings::ChirpSettings;
use super::target::{AccessTokenFuture, AccessTokenResolver, SpeechStreamingTarget};

const SESSION_UPDATE: &str = r#"{"type":"session.update","session":{"audio":{"input":{"format":{"type":"audio/pcm","rate":16000},"transcription":{"model":"chirp_2","language":"en-US"}}}}}"#;
const CONFIGURE_COMMAND: &str = r#"{"kind":"configure","model":"chirp_2","language_codes":["en-US"],"sample_rate_hertz":16000}"#;
const INTERIM: &str = r#"{"kind":"response","speech_event":"none","results":[{"transcript":"hello","is_final":false}],"billed_seconds":0.5}"#;
const CONFIGURED: &str = r#"{"kind":"configured"}"#;

#[derive(Default)]
struct FixedIds {
    events: usize,
    items: usize,
}

impl ChirpIds for FixedIds {
    fn event_id(&mut self) -> String {
        self.events += 1;
        format!("event_fixed_{}", self.events)
    }

    fn item_id(&mut self) -> String {
        self.items += 1;
        format!("item_fixed_{}", self.items)
    }

    fn session_id(&mut self) -> String {
        "sess_fixed".to_owned()
    }
}

struct CountingResolver {
    calls: AtomicUsize,
}

impl CountingResolver {
    fn tracking() -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl AccessTokenResolver for CountingResolver {
    fn resolve_access_token(&self) -> AccessTokenFuture<'_> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(SecretString::from("token")) })
    }
}

struct RecordedTarget {
    api_endpoint: String,
    recognizer: String,
    token: Arc<dyn AccessTokenResolver>,
}

#[derive(Clone, Default)]
struct RecordingFactory {
    targets: Arc<Mutex<Vec<RecordedTarget>>>,
}

struct EmptyBackend;

#[async_trait]
impl RealtimeBackend for EmptyBackend {
    async fn send(&mut self, _message: RealtimeMessage) -> Result<(), WebSocketError> {
        Ok(())
    }

    async fn recv(&mut self, _decode: Option<bool>) -> Result<RealtimeMessage, WebSocketError> {
        Ok(RealtimeMessage::Text(String::new()))
    }

    async fn close(&mut self) -> Result<(), WebSocketError> {
        Ok(())
    }
}

impl ChirpBackendFactory for RecordingFactory {
    type Backend = EmptyBackend;

    fn open(&self, target: SpeechStreamingTarget) -> Self::Backend {
        self.targets
            .lock()
            .expect("the factory lock")
            .push(RecordedTarget {
                api_endpoint: target.api_endpoint,
                recognizer: target.recognizer,
                token: target.token,
            });
        EmptyBackend
    }
}

struct Fixture {
    config: VertexChirpRealtimeConfig<RecordingFactory>,
    resolver: Arc<CountingResolver>,
    targets: Arc<Mutex<Vec<RecordedTarget>>>,
}

fn fixture(location: Option<&str>) -> Fixture {
    let resolver = CountingResolver::tracking();
    let factory = RecordingFactory::default();
    let targets = Arc::clone(&factory.targets);
    let config = VertexChirpRealtimeConfig::new(
        ChirpSettings {
            project: "my-project".to_owned(),
            location: location.map(str::to_owned),
            catalog: BundledCatalog::default().get(),
            limits: ApiLimits::default(),
            ids: Box::new(FixedIds::default()),
        },
        Arc::clone(&resolver),
        factory,
    )
    .expect("a valid config");
    Fixture {
        config,
        resolver,
        targets,
    }
}

fn transform_input() -> RealtimeResponseTransformInput {
    RealtimeResponseTransformInput {
        session_configuration_request: None,
        current_output_item_id: None,
        current_response_id: None,
        current_delta_chunks: None,
        current_item_chunks: None,
        current_conversation_id: None,
        current_delta_type: None,
    }
}

fn append_message(pcm: &[u8]) -> String {
    format!(
        r#"{{"type":"input_audio_buffer.append","audio":"{}"}}"#,
        STANDARD.encode(pcm)
    )
}

fn audio_lengths(commands: &[RealtimeMessage]) -> Vec<usize> {
    commands
        .iter()
        .map(|message| match message {
            RealtimeMessage::Binary(bytes) => bytes.len(),
            RealtimeMessage::Text(_) => panic!("audio chunks are binary"),
        })
        .collect()
}

#[tokio::test]
async fn backend_factory_target() {
    let fixture = fixture(Some("us-central1"));
    let backend = fixture
        .config
        .open_backend("us-central1-speech.googleapis.com", &HeaderMap::new())
        .await
        .expect("an opened backend");
    assert!(backend.is_some());
    let (endpoint, recognizer, resolver) = {
        let targets = fixture.targets.lock().expect("the factory lock");
        assert_eq!(targets.len(), 1);
        (
            targets[0].api_endpoint.clone(),
            targets[0].recognizer.clone(),
            Arc::clone(&targets[0].token),
        )
    };
    assert_eq!(endpoint, "us-central1-speech.googleapis.com");
    assert_eq!(
        recognizer,
        "projects/my-project/locations/us-central1/recognizers/_"
    );
    let token = resolver
        .resolve_access_token()
        .await
        .expect("the attached resolver");
    assert_eq!(token.expose_secret(), "token");
    assert_eq!(fixture.resolver.calls(), 1);
}

#[tokio::test]
async fn no_token_until_audio() {
    let fixture = fixture(None);
    assert_eq!(fixture.resolver.calls(), 0);
    let backend = fixture
        .config
        .open_backend("us-speech.googleapis.com", &HeaderMap::new())
        .await
        .expect("an opened backend");
    assert!(backend.is_some());
    assert_eq!(fixture.resolver.calls(), 0);
    let commands = fixture
        .config
        .transform_realtime_request(SESSION_UPDATE, "chirp_2", None)
        .expect("a configure command");
    assert_eq!(commands.len(), 1);
    let chunks = fixture
        .config
        .transform_realtime_request(&append_message(&[0, 0]), "chirp_2", None)
        .expect("one audio chunk");
    assert_eq!(audio_lengths(&chunks), [2_usize]);
    assert_eq!(fixture.resolver.calls(), 0);
}

#[test]
fn close_usage_once() {
    let fixture = fixture(None);
    fixture
        .config
        .transform_realtime_request(SESSION_UPDATE, "chirp_2", None)
        .expect("a configure command");
    fixture
        .config
        .transform_realtime_response(
            &RealtimeMessage::Text(INTERIM.to_owned()),
            "chirp_2",
            &transform_input(),
        )
        .expect("a transformed frame");
    let usage = fixture
        .config
        .session_close_usage("chirp_2")
        .expect("unreported billing");
    let document = serde_json::to_value(&usage).expect("a usage document");
    assert_eq!(document["type"], "duration");
    assert_eq!(document["seconds"], 0.5_f64);
    assert!(fixture.config.session_close_usage("chirp_2").is_none());
    assert!(
        fixture(None)
            .config
            .session_close_usage("chirp_2")
            .is_none()
    );
}

#[rstest]
#[case::blank("", "the vertex_project '' cannot be used in a URL")]
#[case::parent("..", "the vertex_project '..' cannot be used in a URL")]
#[case::slash("a/b", "the vertex_project 'a/b' cannot be used in a URL")]
#[case::space("a b", "the vertex_project 'a b' cannot be used in a URL")]
#[case::scheme("a:b", "the vertex_project 'a:b' cannot be used in a URL")]
fn a_project_that_could_move_the_request_is_rejected(#[case] project: &str, #[case] message: &str) {
    let error = VertexChirpRealtimeConfig::new(
        ChirpSettings {
            project: project.to_owned(),
            location: None,
            catalog: BundledCatalog::default().get(),
            limits: ApiLimits::default(),
            ids: Box::new(FixedIds::default()),
        },
        CountingResolver::tracking(),
        RecordingFactory::default(),
    )
    .err()
    .expect("an unsafe project");
    assert_eq!(error.to_string(), message);
    assert_eq!(error.status_code(), Some(400));
}

#[rstest]
#[case::uppercase("US")]
#[case::empty("")]
#[case::injection("evil.com#")]
#[case::trailing_space("us-central1 ")]
#[case::dot("us.central1")]
fn a_location_that_could_move_the_host_is_rejected(#[case] location: &str) {
    let error = VertexChirpRealtimeConfig::new(
        ChirpSettings {
            project: "my-project".to_owned(),
            location: Some(location.to_owned()),
            catalog: BundledCatalog::default().get(),
            limits: ApiLimits::default(),
            ids: Box::new(FixedIds::default()),
        },
        CountingResolver::tracking(),
        RecordingFactory::default(),
    )
    .err()
    .expect("an unsafe location");
    assert_eq!(error.to_string(), "invalid vertex_location format");
    assert_eq!(error.status_code(), Some(400));
}

#[rstest]
#[case::default_us(None, None, "us-speech.googleapis.com")]
#[case::regional(Some("us-central1"), None, "us-central1-speech.googleapis.com")]
#[case::global(Some("global"), None, "speech.googleapis.com")]
#[case::custom_path(Some("eu"), Some("https://gateway.example/speech"), "gateway.example")]
#[case::custom_port(Some("eu"), Some("gateway.example:443/ignored"), "gateway.example:443")]
#[case::plain_host(Some("eu"), Some("gateway.example"), "gateway.example")]
#[case::empty_base(Some("eu"), Some(""), "")]
fn complete_url_uses_the_speech_host_or_the_custom_authority(
    #[case] location: Option<&str>,
    #[case] api_base: Option<&str>,
    #[case] expected: &str,
) {
    let fixture = fixture(location);
    assert_eq!(
        fixture
            .config
            .complete_url(api_base, "vertex_ai/chirp_2")
            .unwrap(),
        expected
    );
    assert_eq!(
        fixture.config.get_complete_url(api_base, "chirp_2", None),
        expected
    );
}

#[rstest]
#[case::openai("openai/gpt-4o-transcribe")]
#[case::speech("chirp")]
#[case::gemini("gemini-3.5-transcribe")]
fn complete_url_rejects_models_without_the_speech_catalog_capability(#[case] model: &str) {
    let fixture = fixture(None);
    let error = fixture.config.complete_url(None, model).unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("Unsupported Speech-to-Text streaming model: {model}")
    );
    assert!(matches!(error, ChirpAdapterError::UnsupportedModel { .. }));
    assert_eq!(fixture.config.get_complete_url(None, model, None), "");
}

#[test]
fn headers_pass_through_unchanged() {
    let fixture = fixture(None);
    let headers = HeaderMap::from([(
        "Authorization".to_owned(),
        Json::Text("Bearer token".to_owned()),
    )]);
    let environment = Environment {
        headers: &headers,
        model: "chirp_2",
        api_key: None,
    };
    assert_eq!(
        fixture.config.validate_environment(&environment).unwrap(),
        headers
    );
}

#[rstest]
#[case::configure(r#"{"kind":"configure"}"#, true)]
#[case::blank(r#"{}"#, false)]
#[case::other_kind(r#"{"kind":"finish_turn"}"#, false)]
#[case::number_kind(r#"{"kind":5}"#, false)]
#[case::not_an_object(r#"["configure"]"#, false)]
fn setup_messages_are_the_configure_kind(#[case] message: &str, #[case] expected: bool) {
    let fixture = fixture(None);
    let document = serde_json::from_str::<Json>(message).expect("a JSON document");
    assert_eq!(fixture.config.is_setup_message(&document), expected);
}

#[test]
fn the_first_session_update_configures_and_the_rest_are_ignored() {
    let fixture = fixture(None);
    let commands = fixture
        .config
        .transform_realtime_request(SESSION_UPDATE, "chirp_2", None)
        .unwrap();
    assert_eq!(
        commands,
        [RealtimeMessage::Text(CONFIGURE_COMMAND.to_owned())]
    );
    let invalid = r#"{"type":"session.update","session":{"audio":{"input":{"format":{"type":"audio/opus"}}}}}"#;
    assert!(
        fixture
            .config
            .transform_realtime_request(invalid, "chirp_2", None)
            .unwrap()
            .is_empty()
    );
    assert!(
        fixture
            .config
            .transform_realtime_request(
                r#"{"type":"transcription_session.update","session":{"audio":{}}}"#,
                "chirp_2",
                None
            )
            .unwrap()
            .is_empty()
    );
}

#[test]
fn audio_appends_require_configuration_and_valid_pcm16() {
    let fixture = fixture(None);
    let unconfigured = fixture
        .config
        .transform_realtime_request(&append_message(&[0, 0]), "chirp_2", None)
        .unwrap_err();
    assert_eq!(
        unconfigured.to_string(),
        "session.update must configure the session before audio is sent"
    );
    fixture
        .config
        .transform_realtime_request(SESSION_UPDATE, "chirp_2", None)
        .unwrap();
    let invalid = fixture
        .config
        .transform_realtime_request(
            r#"{"type":"input_audio_buffer.append","audio":"**"}"#,
            "chirp_2",
            None,
        )
        .unwrap_err();
    assert_eq!(invalid.to_string(), "Audio must be valid base64");
    let incomplete = fixture
        .config
        .transform_realtime_request(
            r#"{"type":"input_audio_buffer.append","audio":"AA=="}"#,
            "chirp_2",
            None,
        )
        .unwrap_err();
    assert_eq!(
        incomplete.to_string(),
        "PCM16 audio must contain complete samples"
    );
    let missing = fixture
        .config
        .transform_realtime_request(r#"{"type":"input_audio_buffer.append"}"#, "chirp_2", None)
        .unwrap_err();
    assert_eq!(missing.to_string(), "Audio must be a base64 string");
}

#[rstest]
#[case::empty(0, vec![])]
#[case::just_under(24_998, vec![24_998])]
#[case::at_the_limit(25_000, vec![25_000])]
#[case::just_over(25_001, vec![25_000, 1])]
#[case::two_chunks(50_000, vec![25_000, 25_000])]
fn audio_appends_split_into_25000_byte_chunks(#[case] total: usize, #[case] expected: Vec<usize>) {
    let fixture = fixture(None);
    fixture
        .config
        .transform_realtime_request(SESSION_UPDATE, "chirp_2", None)
        .unwrap();
    let pcm: Vec<u8> = (0..total)
        .map(|index| u8::try_from(index).unwrap_or(u8::MAX))
        .collect();
    let commands = fixture
        .config
        .transform_realtime_request(&append_message(&pcm), "chirp_2", None)
        .unwrap();
    assert_eq!(audio_lengths(&commands), expected);
    let decoded: Vec<u8> = commands
        .iter()
        .filter_map(|message| match message {
            RealtimeMessage::Binary(bytes) => Some(bytes.clone()),
            RealtimeMessage::Text(_) => None,
        })
        .flatten()
        .collect();
    assert_eq!(decoded, pcm);
}

#[rstest]
#[case::commit(r#"{"type":"input_audio_buffer.commit"}"#)]
#[case::end(r#"{"type":"input_audio_buffer.end"}"#)]
#[case::clear(r#"{"type":"input_audio_buffer.clear"}"#)]
fn turn_events_require_a_configured_session(#[case] event: &str) {
    let fixture = fixture(None);
    let error = fixture
        .config
        .transform_realtime_request(event, "chirp_2", None)
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "session.update must configure the session before audio is sent"
    );
}

#[rstest]
#[case::commit(r#"{"type":"input_audio_buffer.commit"}"#, r#"{"kind":"finish_turn"}"#)]
#[case::end(r#"{"type":"input_audio_buffer.end"}"#, r#"{"kind":"finish_turn"}"#)]
#[case::clear(r#"{"type":"input_audio_buffer.clear"}"#, r#"{"kind":"discard_turn"}"#)]
fn turn_events_emit_the_turn_command_after_configuration(
    #[case] event: &str,
    #[case] expected: &str,
) {
    let fixture = fixture(None);
    fixture
        .config
        .transform_realtime_request(SESSION_UPDATE, "chirp_2", None)
        .unwrap();
    let commands = fixture
        .config
        .transform_realtime_request(event, "chirp_2", None)
        .unwrap();
    assert_eq!(commands, [RealtimeMessage::Text(expected.to_owned())]);
}

#[rstest]
#[case::response_create(r#"{"type":"response.create"}"#)]
#[case::session_created(r#"{"type":"session.created"}"#)]
#[case::blank("{}")]
fn unsupported_client_events_are_dropped(#[case] message: &str) {
    let fixture = fixture(None);
    assert!(
        fixture
            .config
            .transform_realtime_request(message, "chirp_2", None)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn non_objects_and_invalid_json_are_refused() {
    let fixture = fixture(None);
    let not_an_object = fixture
        .config
        .transform_realtime_request("[1]", "chirp_2", None)
        .unwrap_err();
    assert_eq!(not_an_object.to_string(), "message must be a JSON object");
    let invalid = fixture
        .config
        .transform_realtime_request("{", "chirp_2", None)
        .unwrap_err();
    assert_eq!(invalid.to_string(), "invalid JSON object");
}

#[test]
fn session_created_uses_the_logging_id_and_default_config() {
    let fixture = fixture(None);
    let event = fixture
        .config
        .session_created_event("vertex_ai/chirp_2", "sess_log", None)
        .expect("a session event");
    let document = serde_json::to_value(&event).expect("a session document");
    assert_eq!(document["type"], "session.created");
    assert_eq!(document["session"]["id"], "sess_log");
    assert_eq!(document["event_id"], "event_fixed_1");
    assert_eq!(
        document["session"]["object"],
        "realtime.transcription_session"
    );
    assert_eq!(document["session"]["type"], "transcription");
    assert_eq!(
        document["session"]["audio"]["input"]["format"]["type"],
        "audio/pcm"
    );
    assert_eq!(
        document["session"]["audio"]["input"]["format"]["rate"],
        24_000_i64
    );
    assert_eq!(
        document["session"]["audio"]["input"]["transcription"]["model"],
        "chirp_2"
    );
    assert!(
        document["session"]["audio"]["input"]["transcription"]["language"].is_null(),
        "the default session config carries no language"
    );
    assert_eq!(
        document["session"]["audio"]["input"]["turn_detection"]["type"],
        "server_vad"
    );
}

#[test]
fn configuration_without_a_prior_session_creates_a_sess_id() {
    let fixture = fixture(None);
    fixture
        .config
        .transform_realtime_request(SESSION_UPDATE, "chirp_2", None)
        .unwrap();
    let result = fixture
        .config
        .transform_realtime_response(
            &RealtimeMessage::Text(CONFIGURED.to_owned()),
            "chirp_2",
            &transform_input(),
        )
        .unwrap();
    let document = serde_json::to_value(&result.response).expect("a response document");
    assert_eq!(document[0]["session"]["id"], "sess_fixed");
    assert_eq!(document[0]["event_id"], "event_fixed_1");
}

#[test]
fn response_transforms_preserve_the_input_state() {
    let fixture = fixture(None);
    fixture
        .config
        .transform_realtime_request(SESSION_UPDATE, "chirp_2", None)
        .unwrap();
    let input = RealtimeResponseTransformInput {
        session_configuration_request: Some("configure".to_owned()),
        current_output_item_id: Some("item_ctx".to_owned()),
        current_response_id: Some("resp_ctx".to_owned()),
        current_delta_chunks: Some(vec![Json::Text("x".to_owned())]),
        current_item_chunks: Some(vec![Json::Text("done".to_owned())]),
        current_conversation_id: Some("conv_ctx".to_owned()),
        current_delta_type: Some(AllDeltaType::Text),
    };
    let result = fixture
        .config
        .transform_realtime_response(
            &RealtimeMessage::Text(CONFIGURED.to_owned()),
            "chirp_2",
            &input,
        )
        .unwrap();
    let expected = RealtimeResponseTypedDict {
        response: result.response.clone(),
        session_configuration_request: input.session_configuration_request.clone(),
        current_output_item_id: input.current_output_item_id.clone(),
        current_response_id: input.current_response_id.clone(),
        current_delta_chunks: input.current_delta_chunks.clone(),
        current_conversation_id: input.current_conversation_id.clone(),
        current_item_chunks: input.current_item_chunks.clone(),
        current_delta_type: input.current_delta_type,
    };
    assert_eq!(result, expected);
    let document = serde_json::to_value(&result.response).expect("a response document");
    assert_eq!(document[0]["type"], "session.created");
}

#[test]
fn response_events_take_their_item_ids_from_the_injected_source() {
    let fixture = fixture(None);
    fixture
        .config
        .transform_realtime_request(SESSION_UPDATE, "chirp_2", None)
        .unwrap();
    let result = fixture
        .config
        .transform_realtime_response(
            &RealtimeMessage::Text(INTERIM.to_owned()),
            "chirp_2",
            &transform_input(),
        )
        .unwrap();
    let document = serde_json::to_value(&result.response).expect("a response document");
    assert!(
        document
            .as_array()
            .expect("an event list")
            .iter()
            .any(|event| event["item_id"] == "item_fixed_1"),
        "{document}"
    );
}

#[test]
fn uuid_ids_carry_their_prefixes_and_differ_per_call() {
    let mut ids = UuidChirpIds::default();
    let event = ids.event_id();
    let item = ids.item_id();
    let session = ids.session_id();
    assert_eq!(event.len(), "event_".len() + 32);
    assert!(event.starts_with("event_"));
    assert_eq!(item.len(), "item_".len() + 32);
    assert!(item.starts_with("item_"));
    assert_eq!(session.len(), "sess_".len() + 32);
    assert!(session.starts_with("sess_"));
    assert_ne!(ids.event_id(), event);
    assert_ne!(ids.item_id(), item);
    assert_ne!(ids.session_id(), session);
}

#[test]
fn the_decoder_reads_backend_events() {
    assert_eq!(
        decode_chirp_backend_event(CONFIGURED.as_bytes(), ApiLimits::default()).unwrap(),
        ChirpBackendEvent::Configured
    );
    match decode_chirp_backend_event(INTERIM.as_bytes(), ApiLimits::default()).unwrap() {
        ChirpBackendEvent::Response(response) => {
            assert_eq!(response.billed_seconds, Decimal::new(5, 1));
        }
        other => panic!("a response frame, not {other:?}"),
    }
}

#[rstest]
#[case::truncated("{")]
#[case::unknown_kind(r#"{"kind":"unknown"}"#)]
fn the_decoder_rejects_malformed_events(#[case] payload: &str) {
    let error = decode_chirp_backend_event(payload.as_bytes(), ApiLimits::default()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "the Speech-to-Text streaming event is not a valid backend event"
    );
    assert!(matches!(error, ChirpAdapterError::Event { .. }));
}

#[test]
fn the_decoder_rejects_an_oversized_event() {
    let limits = ApiLimits {
        max_json_bytes: 1,
        max_event_bytes: 4,
        max_items: 1,
    };
    let error = decode_chirp_backend_event(CONFIGURED.as_bytes(), limits).unwrap_err();
    assert_eq!(
        error.to_string(),
        "the provider event is 21 bytes; the maximum is 4"
    );
}

#[rstest]
fn replays_the_chirp_backend_event_corpus(
    #[files("fuzz/corpus/chirp_backend_event/*")] path: PathBuf,
) {
    let bytes = fs::read(&path).expect("a corpus file");
    let decoded = decode_chirp_backend_event(&bytes, ApiLimits::default());
    let rejected = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("rejected-"));
    if rejected {
        assert!(decoded.is_err(), "{} must be rejected", path.display());
        return;
    }
    assert!(decoded.is_ok(), "{} must decode", path.display());
}
