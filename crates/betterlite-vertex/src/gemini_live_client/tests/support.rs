#![cfg(test)]

use std::fs;
use std::sync::Arc;

use betterlite_config::sections::vertex_schema_shaping::Settings as SchemaSettings;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_prompts::arguments::json::Json;
use tempfile::NamedTempFile;
use tracing::subscriber::with_default;

use crate::gemini_live_client::decode::{ClientEvent, decode_gemini_live_client_event};
use crate::gemini_live_client::error::LiveClientError;
use crate::gemini_live_client::state::LiveSessionState;
use crate::gemini_live_client::transform::{FunctionResponseId, LiveClient};
use crate::gemini_live_setup::settings::LiveSetupSettings;

const CATALOG: &str = r#"{"gemini-2.5-flash": {"litellm_provider": "gemini", "mode": "realtime"}}"#;

pub(crate) fn decode(source: &str) -> ClientEvent {
    decode_bytes(source.as_bytes(), usize::MAX).expect("the event decodes")
}

pub(crate) fn decode_bytes(bytes: &[u8], limit: usize) -> Result<ClientEvent, LiveClientError> {
    decode_gemini_live_client_event(bytes, limit)
}

pub(crate) fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("the JSON parses")
}

pub(crate) fn assert_messages(encoded: &[String], expected: &[&str]) {
    let actual: Vec<Json> = encoded.iter().map(|message| json(message)).collect();
    let expected: Vec<Json> = expected.iter().map(|source| json(source)).collect();
    assert_eq!(actual, expected);
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

pub(crate) struct Session {
    client: LiveClient,
    catalog: ModelCatalog,
    schema: SchemaSettings,
}

impl Session {
    pub(crate) fn new() -> Self {
        let catalog =
            ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject).expect("the catalog");
        Self {
            client: LiveClient {
                state: LiveSessionState::default(),
                function_response_id: FunctionResponseId::Include,
            },
            catalog,
            schema: SchemaSettings::default(),
        }
    }

    pub(crate) fn send(&mut self, source: &str) -> Vec<String> {
        self.send_with_request(source, None)
    }

    pub(crate) fn send_with_request(&mut self, source: &str, request: Option<&str>) -> Vec<String> {
        self.try_send_with_request(source, request)
            .expect("the event transforms")
    }

    pub(crate) fn try_send(&mut self, source: &str) -> Result<Vec<String>, LiveClientError> {
        self.try_send_with_request(source, None)
    }

    pub(crate) fn try_send_with_request(
        &mut self,
        source: &str,
        request: Option<&str>,
    ) -> Result<Vec<String>, LiveClientError> {
        let settings = LiveSetupSettings {
            catalog: &self.catalog,
            schema: &self.schema,
            defer_setup: false,
        };
        self.client.transform_realtime_request(
            &decode(source),
            "gemini-2.5-flash",
            request,
            &settings,
        )
    }

    pub(crate) fn without_response_ids() -> Self {
        let mut session = Self::new();
        session.client.function_response_id = FunctionResponseId::Omit;
        session
    }

    pub(crate) fn register(&mut self, call_id: &str, name: &str) {
        self.client.state.tool_call_names.insert(call_id, name);
    }

    pub(crate) const fn bytes(&self) -> u64 {
        self.client.state.unbilled_input_audio_bytes
    }

    pub(crate) const fn state(&self) -> &LiveSessionState {
        &self.client.state
    }
}
