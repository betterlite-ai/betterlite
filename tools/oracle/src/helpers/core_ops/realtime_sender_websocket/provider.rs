use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_core::api::realtime_contracts::config::{BaseRealtimeConfig, Environment};
use betterlite_core::api::realtime_contracts::error::{HeaderMap, WebSocketError};
use betterlite_core::api::realtime_contracts::events::{
    AudioTranscriptionUsage, RealtimeResponseTransformInput, RealtimeResponseTypedDict,
};
use betterlite_core::api::realtime_contracts::transport::{
    RealtimeEventNormalizer, RealtimeMessage,
};
use betterlite_core::api::realtime_guardrail_policy::RealtimeGuardrailCallback;
use betterlite_core::api::realtime_session_state::RealtimeProviderConfig;
use betterlite_core::api::request_destination::RequestData;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::json;
use thiserror::Error;

use super::input::{Frame, GuardrailDocument, ProviderDocument};
use super::transport::{Locked as _, Timeline, WireBackend, capture};

#[derive(Debug, Error)]
pub(crate) enum ScriptError {
    #[error("the scripted provider transformation failed")]
    Injected,
    #[error("the scripted provider capture failed")]
    Capture {
        #[source]
        source: WebSocketError,
    },
}

pub(crate) struct ScriptedProvider {
    requires_session_configuration: bool,
    setup: Vec<Json>,
    content: Vec<Json>,
    transforms: Mutex<VecDeque<Vec<Frame>>>,
    fail_transform: bool,
    timeline: Timeline,
    capture: Arc<Mutex<usize>>,
}

impl ScriptedProvider {
    pub(crate) fn new(
        document: &ProviderDocument,
        timeline: Timeline,
        capture: Arc<Mutex<usize>>,
    ) -> Self {
        Self {
            requires_session_configuration: document.requires_session_configuration,
            setup: document.setup.clone(),
            content: document.content.clone(),
            transforms: Mutex::new(document.transforms.iter().cloned().collect()),
            fail_transform: document.fail_transform,
            timeline,
            capture,
        }
    }
}

#[async_trait]
impl BaseRealtimeConfig for ScriptedProvider {
    type Error = ScriptError;
    type Backend = WireBackend;

    fn validate_environment(
        &self,
        _environment: &Environment<'_>,
    ) -> Result<HeaderMap, Self::Error> {
        Ok(HeaderMap::new())
    }

    fn transform_realtime_request(
        &self,
        message: &str,
        model: &str,
        session_configuration_request: Option<&str>,
    ) -> Result<Vec<RealtimeMessage>, Self::Error> {
        capture(
            &self.capture,
            &[
                message.len(),
                model.len(),
                session_configuration_request.map_or(0, str::len),
            ],
        )
        .map_err(|source| ScriptError::Capture { source })?;
        let decoded = serde_json::from_str::<Json>(message)
            .unwrap_or_else(|_error| Json::Text(message.to_owned()));
        self.timeline.locked().push(json!({
            "transform": {
                "message": decoded,
                "model": model,
                "session_configuration_request": session_configuration_request,
            }
        }));
        if self.fail_transform {
            return Err(ScriptError::Injected);
        }
        let frames = self.transforms.locked().pop_front().unwrap_or_default();
        Ok(frames.into_iter().map(Frame::message).collect())
    }

    async fn pace_backend_send(&self, _message: &[u8]) {
        self.timeline.locked().push(json!("pace"));
    }

    fn is_setup_message(&self, message: &Json) -> bool {
        self.setup.iter().any(|candidate| candidate == message)
    }

    fn is_content_message(&self, message: &Json) -> bool {
        self.content.iter().any(|candidate| candidate == message)
    }

    fn requires_session_configuration(&self) -> bool {
        self.requires_session_configuration
    }

    fn transform_realtime_response(
        &self,
        _message: &RealtimeMessage,
        _model: &str,
        _input: &RealtimeResponseTransformInput,
    ) -> Result<RealtimeResponseTypedDict, Self::Error> {
        Ok(RealtimeResponseTypedDict {
            response: Json::Object(IndexMap::new()),
            current_output_item_id: None,
            current_response_id: None,
            current_delta_chunks: None,
            current_conversation_id: None,
            current_item_chunks: None,
            current_delta_type: None,
            session_configuration_request: None,
        })
    }
}

// The connection state only asks its provider config whether setup is deferred and whether closing bills usage
pub(crate) struct ProviderView {
    pub(crate) requires_session_configuration: bool,
}

impl RealtimeProviderConfig for ProviderView {
    fn requires_session_configuration(&self) -> bool {
        self.requires_session_configuration
    }

    fn unbilled_usage_on_close(&self, _model: &str) -> Option<AudioTranscriptionUsage> {
        None
    }
}

pub(crate) struct Guard {
    custom: bool,
    runs_for: Vec<GuardrailEventHooks>,
}

impl Guard {
    pub(crate) fn documents(documents: &[GuardrailDocument]) -> Vec<Self> {
        documents
            .iter()
            .map(|document| Self {
                custom: document.custom,
                runs_for: document.runs_for.clone(),
            })
            .collect()
    }
}

impl RealtimeGuardrailCallback for Guard {
    type Error = ScriptError;

    fn is_custom_guardrail(&self) -> bool {
        self.custom
    }

    fn should_run_guardrail(
        &self,
        _data: &RequestData,
        event: GuardrailEventHooks,
    ) -> Result<bool, Self::Error> {
        Ok(self.runs_for.contains(&event))
    }
}

pub(crate) struct Normalizer;

impl RealtimeEventNormalizer for Normalizer {
    fn should_drop(&self, event: &Json) -> bool {
        let Json::Object(fields) = event else {
            return false;
        };
        fields.get("type") == Some(&Json::Text(String::from("conversation.item.added")))
    }

    fn normalize(&self, event: Json) -> Json {
        let Json::Object(mut fields) = event else {
            return event;
        };
        if fields.get("type") == Some(&Json::Text(String::from("response.output_text.done"))) {
            fields.insert(
                String::from("type"),
                Json::Text(String::from("response.text.done")),
            );
        }
        Json::Object(fields)
    }

    fn patch_outgoing_session(&self, session: Json) -> Json {
        session
    }
}
