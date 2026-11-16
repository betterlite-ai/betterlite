#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::sync::Arc;

use betterlite_config::sections::vertex_schema_shaping::Settings as SchemaSettings;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_prompts::arguments::json::Json;
use secrecy::SecretString;
use serde_json::Number;
use tempfile::NamedTempFile;
use tracing::subscriber::with_default;

use crate::gemini_live_client::decode::decode_gemini_live_client_event;
use crate::gemini_live_client::state::LiveSessionState;
use crate::gemini_live_client::transform::{FunctionResponseId, LiveClient};
use crate::gemini_live_setup::decode::decode_gemini_live_setup;
use crate::gemini_live_setup::settings::LiveSetupSettings;
use crate::gemini_live_setup::wire::AutomaticActivityDetection;

use super::client::VertexLiveClient;
use super::error::VertexLiveError;
use super::policy::VertexLivePolicy;
use super::session::VertexLiveInput;

const POLICY_TOKEN: &str = "ya29.test-token";
const RPC_PATH: &str = "/ws/google.cloud.aiplatform.v1.LlmBidiService/BidiGenerateContent";

const CATALOG: &str = r#"{
    "gemini-2.5-flash": {"litellm_provider": "vertex_ai", "mode": "realtime"},
    "transcribe-live": {"litellm_provider": "vertex_ai", "mode": "audio_transcription"}
}"#;

fn policy(project: &str, region: &str) -> VertexLivePolicy {
    VertexLivePolicy {
        project: project.to_owned(),
        region: region.to_owned(),
        access_token: SecretString::from(POLICY_TOKEN.to_owned()),
    }
}

fn catalog() -> ModelCatalog {
    ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject).expect("the test catalog parses")
}

fn parse_json(source: &str) -> Json {
    serde_json::from_str(source).expect("the literal is JSON")
}

fn client_messages(
    policy: &VertexLivePolicy,
    catalog: &ModelCatalog,
    message: &str,
    model: &str,
    request: Option<&str>,
) -> Vec<String> {
    let schema = SchemaSettings::default();
    let input = VertexLiveInput {
        policy,
        catalog,
        schema: &schema,
    };
    let mut client = VertexLiveClient::default();
    client
        .transform_realtime_request(message.as_bytes(), usize::MAX, model, request, &input)
        .expect("the client message transforms")
}

fn session_message(session: &str) -> String {
    format!(r#"{{"type": "session.update", "session": {session}}}"#)
}

fn detection(messages: &[String]) -> AutomaticActivityDetection {
    let setup = decode_gemini_live_setup(&messages[0]).expect("the setup decodes");
    setup
        .realtime_input_config
        .and_then(|config| config.automatic_activity_detection)
        .expect("automatic activity detection is present")
}

fn with_warnings<F>(run: F) -> String
where
    F: FnOnce(),
{
    let log = NamedTempFile::new().expect("a log file");
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().expect("the log reopens")))
        .finish();
    with_default(subscriber, run);
    fs::read_to_string(log.path()).expect("the log reads")
}

#[test]
fn global_location_uses_the_global_host() {
    assert_eq!(
        policy("my-project", "global").complete_url(None),
        format!("wss://aiplatform.googleapis.com{RPC_PATH}")
    );
}

#[test]
fn regional_location_uses_the_regional_host() {
    assert_eq!(
        policy("my-project", "europe-west4").complete_url(None),
        format!("wss://europe-west4-aiplatform.googleapis.com{RPC_PATH}")
    );
}

#[test]
fn custom_api_base_switches_https_to_wss() {
    assert_eq!(
        policy("my-project", "global").complete_url(Some("https://gateway.example.com/vertex///")),
        format!("wss://gateway.example.com/vertex{RPC_PATH}")
    );
}

#[test]
fn custom_api_base_switches_http_to_ws() {
    assert_eq!(
        policy("my-project", "global").complete_url(Some("http://localhost:8080/")),
        format!("ws://localhost:8080{RPC_PATH}")
    );
}

#[test]
fn environment_forces_bearer_and_adds_the_project_header() {
    let headers = BTreeMap::from([
        ("content-type".to_owned(), "application/json".to_owned()),
        ("Authorization".to_owned(), "stale".to_owned()),
    ]);
    let environment = policy("my-project", "global").environment(&headers);
    assert_eq!(
        environment.get("Authorization").map(String::as_str),
        Some("Bearer ya29.test-token")
    );
    assert_eq!(
        environment.get("x-goog-user-project").map(String::as_str),
        Some("my-project")
    );
    assert_eq!(
        environment.get("content-type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(environment.len(), 3);
}

#[test]
fn environment_omits_the_project_header_for_an_empty_project() {
    let environment = policy("", "global").environment(&BTreeMap::new());
    assert!(!environment.contains_key("x-goog-user-project"));
    assert_eq!(
        environment.get("Authorization").map(String::as_str),
        Some("Bearer ya29.test-token")
    );
}

#[test]
fn default_setup_carries_vertex_defaults() {
    let request = policy("my-project", "us-central1")
        .session_configuration_request("gemini-2.5-flash")
        .expect("the setup builds");
    assert_eq!(
        parse_json(&request),
        parse_json(
            r#"{"setup": {
                "model": "projects/my-project/locations/us-central1/publishers/google/models/gemini-2.5-flash",
                "generationConfig": {"responseModalities": ["AUDIO"]},
                "realtimeInputConfig": {"automaticActivityDetection": {"disabled": false, "silenceDurationMs": 800}},
                "inputAudioTranscription": {},
                "outputAudioTranscription": {}
            }}"#
        )
    );
}

#[test]
fn global_default_setup_keeps_the_global_location_in_the_model_path() {
    let request = policy("my-project", "global")
        .session_configuration_request("gemini-2.5-flash")
        .expect("the setup builds");
    let setup = decode_gemini_live_setup(&request).expect("the setup decodes");
    assert_eq!(
        setup.model.as_deref(),
        Some("projects/my-project/locations/global/publishers/google/models/gemini-2.5-flash")
    );
}

#[test]
fn deferred_setup_keeps_audio_for_a_text_only_model() {
    // LiteLLM hardcodes AUDIO here and only the session.update path runs the catalog coercion
    let request = policy("my-project", "us-central1")
        .session_configuration_request("transcribe-live")
        .expect("the setup builds");
    let setup = decode_gemini_live_setup(&request).expect("the setup decodes");
    assert_eq!(
        setup.generation_config.response_modalities,
        Some(vec!["AUDIO".to_owned()])
    );
}

#[test]
fn first_session_update_sends_the_vertex_setup() {
    let messages = client_messages(
        &policy("my-project", "europe-west4"),
        &catalog(),
        &session_message(
            r#"{"instructions": "Be brief.", "temperature": 0.3, "modalities": ["text"], "turn_detection": {"type": "server_vad", "silence_duration_ms": 500}}"#,
        ),
        "gemini-2.5-flash",
        None,
    );
    assert_eq!(
        parse_json(&messages[0]),
        parse_json(
            r#"{"setup": {
                "model": "projects/my-project/locations/europe-west4/publishers/google/models/gemini-2.5-flash",
                "generationConfig": {"temperature": 0.3, "responseModalities": ["TEXT"]},
                "systemInstruction": {"role": "user", "parts": [{"text": "Be brief."}]},
                "realtimeInputConfig": {"automaticActivityDetection": {"disabled": false, "silenceDurationMs": 500}},
                "inputAudioTranscription": {},
                "outputAudioTranscription": {}
            }}"#
        )
    );
}

#[test]
fn partial_turn_detection_keeps_its_values_and_enables_detection() {
    let messages = client_messages(
        &policy("my-project", "us-central1"),
        &catalog(),
        &session_message(
            r#"{"turn_detection": {"prefix_padding_ms": 100, "silence_duration_ms": 500}}"#,
        ),
        "gemini-2.5-flash",
        None,
    );
    let detection = detection(&messages);
    assert_eq!(detection.disabled, Some(false));
    assert_eq!(
        detection.prefix_padding_ms,
        Some(Json::Number(Number::from(100_u64)))
    );
    assert_eq!(
        detection.silence_duration_ms,
        Some(Json::Number(Number::from(500_u64)))
    );
}

#[test]
fn missing_turn_detection_gets_enabled_detection_and_the_silence_default() {
    let messages = client_messages(
        &policy("my-project", "us-central1"),
        &catalog(),
        &session_message(r#"{"instructions": "Be brief."}"#),
        "gemini-2.5-flash",
        None,
    );
    let detection = detection(&messages);
    assert_eq!(detection.disabled, Some(false));
    assert_eq!(
        detection.silence_duration_ms,
        Some(Json::Number(Number::from(800_u64)))
    );
}

#[test]
fn explicit_false_create_response_keeps_detection_disabled() {
    let messages = client_messages(
        &policy("my-project", "us-central1"),
        &catalog(),
        &session_message(r#"{"turn_detection": {"type": "server_vad", "create_response": false}}"#),
        "gemini-2.5-flash",
        None,
    );
    let detection = detection(&messages);
    assert_eq!(detection.disabled, Some(true));
    assert_eq!(
        detection.silence_duration_ms,
        Some(Json::Number(Number::from(800_u64)))
    );
}

#[test]
fn nested_audio_create_response_false_keeps_detection_disabled() {
    let messages = client_messages(
        &policy("my-project", "us-central1"),
        &catalog(),
        &session_message(
            r#"{"audio": {"input": {"turn_detection": {"type": "server_vad", "create_response": false}}}}"#,
        ),
        "gemini-2.5-flash",
        None,
    );
    assert_eq!(detection(&messages).disabled, Some(true));
}

#[test]
fn explicit_true_create_response_enables_detection() {
    let messages = client_messages(
        &policy("my-project", "us-central1"),
        &catalog(),
        &session_message(r#"{"turn_detection": {"type": "server_vad", "create_response": true}}"#),
        "gemini-2.5-flash",
        None,
    );
    assert_eq!(detection(&messages).disabled, Some(false));
}

#[test]
fn semantic_vad_without_create_response_enables_detection() {
    let messages = client_messages(
        &policy("my-project", "us-central1"),
        &catalog(),
        &session_message(r#"{"turn_detection": {"type": "semantic_vad"}}"#),
        "gemini-2.5-flash",
        None,
    );
    let detection = detection(&messages);
    assert_eq!(detection.disabled, Some(false));
    assert_eq!(
        detection.silence_duration_ms,
        Some(Json::Number(Number::from(800_u64)))
    );
}

#[test]
fn text_only_model_modality_is_coerced() {
    let messages = client_messages(
        &policy("my-project", "us-central1"),
        &catalog(),
        &session_message(r#"{"modalities": ["audio"]}"#),
        "transcribe-live",
        None,
    );
    let setup = decode_gemini_live_setup(&messages[0]).expect("the setup decodes");
    assert_eq!(
        setup.generation_config.response_modalities,
        Some(vec!["TEXT".to_owned()])
    );
}

#[test]
fn later_guardrail_update_is_dropped_with_a_warning() {
    let policy = policy("my-project", "us-central1");
    let catalog = catalog();
    let logged = with_warnings(|| {
        let messages = client_messages(
            &policy,
            &catalog,
            &session_message(
                r#"{"turn_detection": {"type": "server_vad", "create_response": false}}"#,
            ),
            "gemini-2.5-flash",
            Some("stored"),
        );
        assert!(messages.is_empty());
    });
    assert!(logged.contains("WARN"));
    assert!(logged.contains("rejects a second setup message"));
}

#[test]
fn later_nested_guardrail_update_is_dropped_with_a_warning() {
    let policy = policy("my-project", "us-central1");
    let catalog = catalog();
    let logged = with_warnings(|| {
        let messages = client_messages(
            &policy,
            &catalog,
            &session_message(
                r#"{"audio": {"input": {"turn_detection": {"create_response": false}}}}"#,
            ),
            "gemini-2.5-flash",
            Some("stored"),
        );
        assert!(messages.is_empty());
    });
    assert!(logged.contains("WARN"));
    assert!(logged.contains("rejects a second setup message"));
}

#[test]
fn later_update_without_a_guardrail_is_dropped_without_a_warning() {
    let policy = policy("my-project", "us-central1");
    let catalog = catalog();
    let logged = with_warnings(|| {
        let messages = client_messages(
            &policy,
            &catalog,
            &session_message(r#"{"instructions": "late"}"#),
            "gemini-2.5-flash",
            Some("stored"),
        );
        assert!(messages.is_empty());
    });
    assert!(!logged.contains("WARN"));
}

#[test]
fn audio_append_uses_the_vertex_pcm_rate() {
    let messages = client_messages(
        &policy("my-project", "us-central1"),
        &catalog(),
        r#"{"type": "input_audio_buffer.append", "audio": "AAAA"}"#,
        "gemini-2.5-flash",
        None,
    );
    assert_eq!(
        parse_json(&messages[0]),
        parse_json(
            r#"{"realtimeInput": {"audio": {"mimeType": "audio/pcm;rate=24000", "data": "AAAA"}}}"#
        )
    );
}

#[test]
fn commit_event_delegates_to_the_studio_transformer() {
    let messages = client_messages(
        &policy("my-project", "us-central1"),
        &catalog(),
        r#"{"type": "input_audio_buffer.commit"}"#,
        "gemini-2.5-flash",
        None,
    );
    assert_eq!(
        parse_json(&messages[0]),
        parse_json(r#"{"realtimeInput": {"audioStreamEnd": true}}"#)
    );
}

#[test]
fn function_responses_omit_the_call_id() {
    let message = r#"{"type": "conversation.item.create", "item": {"type": "function_call_output", "call_id": "call-1", "output": "{\"temp\": 21}"}}"#;
    let messages = client_messages(
        &policy("my-project", "us-central1"),
        &catalog(),
        message,
        "gemini-2.5-flash",
        None,
    );
    let response = function_response(&messages[0]);
    let fields = response
        .as_object()
        .expect("the function response is an object");
    assert_eq!(fields.get("id"), None);
    assert_eq!(fields.get("response"), Some(&parse_json(r#"{"temp": 21}"#)));
}

#[test]
fn the_studio_transformer_keeps_the_call_id() {
    let message = r#"{"type": "conversation.item.create", "item": {"type": "function_call_output", "call_id": "call-1", "output": "{}"}}"#;
    let catalog = catalog();
    let schema = SchemaSettings::default();
    let settings = LiveSetupSettings {
        catalog: &catalog,
        schema: &schema,
        defer_setup: false,
    };
    let event =
        decode_gemini_live_client_event(message.as_bytes(), usize::MAX).expect("the item decodes");
    let mut studio = LiveClient {
        state: LiveSessionState::default(),
        function_response_id: FunctionResponseId::Include,
    };
    let messages = studio
        .transform_realtime_request(&event, "gemini-2.5-flash", None, &settings)
        .expect("the item transforms");
    let response = function_response(&messages[0]);
    assert_eq!(
        response.as_object().and_then(|fields| fields.get("id")),
        Some(&Json::Text("call-1".to_owned()))
    );
}

#[test]
fn invalid_json_is_rejected() {
    let catalog = catalog();
    let schema = SchemaSettings::default();
    let input = VertexLiveInput {
        policy: &policy("my-project", "us-central1"),
        catalog: &catalog,
        schema: &schema,
    };
    let mut client = VertexLiveClient::default();
    let error = client
        .transform_realtime_request(b"not json", usize::MAX, "gemini-2.5-flash", None, &input)
        .expect_err("invalid JSON is rejected");
    assert!(matches!(error, VertexLiveError::Client(_)));
}

fn function_response(message: &str) -> Json {
    let document = parse_json(message);
    let tool_response = document
        .as_object()
        .and_then(|object| object.get("toolResponse"))
        .expect("the message is a toolResponse");
    let responses = tool_response
        .as_object()
        .and_then(|object| object.get("functionResponses"))
        .expect("functionResponses is present");
    let Json::List(responses) = responses.clone() else {
        panic!("functionResponses is a list");
    };
    let Some(response) = responses.first() else {
        panic!("functionResponses carries one response");
    };
    response.clone()
}
