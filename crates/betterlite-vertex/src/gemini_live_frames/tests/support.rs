#![cfg(test)]

use betterlite_core::api::realtime_contracts::transport::RealtimeMessage;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::Number;
use uuid::Uuid;

use crate::gemini_live_client::state::LiveSessionState;
use crate::gemini_live_frames::decode::{LiveFrame, decode_gemini_live_frame};
use crate::gemini_live_frames::error::LiveFrameError;
use crate::gemini_live_frames::state::CallerState;
use crate::gemini_live_frames::transform::{ResponseContext, transform_realtime_response};

pub(crate) const LIVE_MODEL: &str = "gemini-2.5-flash";
pub(crate) const TRANSCRIBE_MODEL: &str = "gemini/gemini-3.5-transcribe-live";
const UUID_STEP: u128 = 0x0000_0001_0000_0001_0000_0001_0000_0001;
const CATALOG: &str = r#"{
    "gemini-2.5-flash": {"litellm_provider": "gemini", "mode": "realtime"},
    "gemini/gemini-3.5-transcribe-live": {"litellm_provider": "gemini", "mode": "audio_transcription"}
}"#;

pub(crate) struct Live {
    pub(crate) caller: CallerState,
    pub(crate) session: LiveSessionState,
    pub(crate) model: &'static str,
    catalog: ModelCatalog,
    drawn: u128,
}

impl Live {
    pub(crate) fn new(model: &'static str) -> Self {
        Self {
            caller: CallerState::default(),
            session: LiveSessionState::default(),
            model,
            catalog: catalog(),
            drawn: 0,
        }
    }

    pub(crate) fn with_unbilled(model: &'static str, bytes: u64) -> Self {
        let mut live = Self::new(model);
        live.session.unbilled_input_audio_bytes = bytes;
        live
    }

    pub(crate) fn frame(&mut self, text: &str) -> Result<Vec<Json>, LiveFrameError> {
        let message = RealtimeMessage::Text(text.to_owned());
        let frame = decode_gemini_live_frame(&message, usize::MAX)?;
        self.transform(frame)
    }

    pub(crate) fn transform(&mut self, mut frame: LiveFrame) -> Result<Vec<Json>, LiveFrameError> {
        let mut drawn = self.drawn;
        let context = ResponseContext {
            model: self.model,
            logging_session_id: "session-log",
            catalog: &self.catalog,
        };
        let answer = {
            let mut uuids = || {
                drawn = drawn.saturating_add(1);
                Uuid::from_u128(drawn.saturating_mul(UUID_STEP))
            };
            transform_realtime_response(
                &mut frame,
                &self.caller,
                &mut self.session,
                &context,
                &mut uuids,
            )?
        };
        self.drawn = drawn;
        self.caller = answer.state;
        Ok(answer.events)
    }
}

pub(crate) fn catalog() -> ModelCatalog {
    ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject).expect("the test catalog parses")
}

pub(crate) fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("the JSON value parses")
}

pub(crate) fn list(source: &str) -> Vec<Json> {
    serde_json::from_str(source).expect("the JSON list parses")
}

pub(crate) fn object(source: &str) -> Object {
    serde_json::from_str(source).expect("the JSON object parses")
}

pub(crate) fn draw(count: u128) -> String {
    Uuid::from_u128(count.saturating_mul(UUID_STEP)).to_string()
}

pub(crate) fn types(events: &[Json]) -> Vec<&str> {
    events
        .iter()
        .map(|event| {
            event
                .as_object()
                .and_then(|fields| fields.get("type"))
                .and_then(Json::as_str)
                .expect("an event names its type")
        })
        .collect()
}

pub(crate) fn field<'events>(events: &'events [Json], index: usize, name: &str) -> &'events Json {
    events
        .get(index)
        .and_then(Json::as_object)
        .and_then(|fields| fields.get(name))
        .expect("the event carries the field")
}

pub(crate) fn lookup<'value>(value: &'value Json, path: &[&str]) -> &'value Json {
    path.iter().fold(value, |current, key| {
        current
            .as_object()
            .and_then(|fields| fields.get(*key))
            .expect("the usage path is present")
    })
}

pub(crate) fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

pub(crate) fn text(value: String) -> Json {
    Json::Text(value)
}
