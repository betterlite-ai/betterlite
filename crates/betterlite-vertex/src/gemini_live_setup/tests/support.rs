#![cfg(test)]

use std::fs;
use std::sync::Arc;

use betterlite_config::sections::vertex_schema_shaping::Settings as SchemaSettings;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_prompts::arguments::json::Json;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tempfile::NamedTempFile;
use tracing::subscriber::with_default;

use crate::gemini_live_setup::automatic::session_configuration_request;
use crate::gemini_live_setup::created::{SessionCreatedEvent, session_created};
use crate::gemini_live_setup::session::SessionUpdate;
use crate::gemini_live_setup::settings::LiveSetupSettings;
use crate::gemini_live_setup::setup::{map_session, transform_session_update};
use crate::gemini_live_setup::turn_detection::TurnDetection;
use crate::gemini_live_setup::wire::LiveSetup;

const CATALOG: &str = r#"{
    "gemini-2.5-flash": {"litellm_provider": "gemini", "mode": "realtime"},
    "gemini-2.5-flash-native-audio-preview-09-2025": {"litellm_provider": "gemini", "mode": "realtime", "gemini_native_audio": true},
    "audio-only-live": {"litellm_provider": "gemini", "mode": "realtime", "gemini_audio_only_live": true},
    "b/c": {"litellm_provider": "gemini", "mode": "audio_transcription"},
    "both-model": {"litellm_provider": "gemini", "mode": "audio_transcription"},
    "gemini/both-model": {"litellm_provider": "gemini", "mode": "realtime", "gemini_audio_only_live": true},
    "gemini/gemini-3.5-transcribe-live": {"litellm_provider": "gemini", "mode": "audio_transcription"},
    "vertex_ai/gemini-3.5-transcribe-live": {"litellm_provider": "vertex_ai", "mode": "audio_transcription"}
}"#;

pub(crate) fn parse<T>(source: &str) -> T
where
    T: DeserializeOwned,
{
    serde_json::from_str(source).expect("the fixture decodes")
}

pub(crate) fn json(source: &str) -> Json {
    parse(source)
}

pub(crate) fn as_json<T>(value: &T) -> Json
where
    T: Serialize + ?Sized,
{
    let text = serde_json::to_string(value).expect("the value serializes");
    serde_json::from_str(&text).expect("the value is JSON")
}

pub(crate) fn catalog() -> ModelCatalog {
    ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject).expect("the test catalog parses")
}

pub(crate) fn session(source: &str) -> SessionUpdate {
    parse(source)
}

pub(crate) fn turn_detection(source: &str) -> TurnDetection {
    parse(source)
}

pub(crate) fn mapped(source: &str) -> LiveSetup {
    map_session(&session(source), &SchemaSettings::default()).expect("the session maps")
}

pub(crate) fn transform(source: &str, model: &str, request: Option<&str>) -> Vec<String> {
    let catalog = catalog();
    let schema = SchemaSettings::default();
    let settings = LiveSetupSettings {
        catalog: &catalog,
        schema: &schema,
        defer_setup: false,
    };
    transform_session_update(&session(source), model, request, &settings)
        .expect("the update transforms")
}

pub(crate) fn automatic(model: &str, defer_setup: bool) -> Option<String> {
    let catalog = catalog();
    let schema = SchemaSettings::default();
    let settings = LiveSetupSettings {
        catalog: &catalog,
        schema: &schema,
        defer_setup,
    };
    session_configuration_request(model, &settings).expect("the automatic request builds")
}

pub(crate) fn created(
    model: &str,
    logging_session_id: &str,
    request: Option<&str>,
    event_id: &str,
) -> SessionCreatedEvent {
    session_created(model, logging_session_id, request, event_id).expect("the event builds")
}

pub(crate) fn with_warnings<F>(run: F) -> String
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
