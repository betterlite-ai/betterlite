use std::convert::Infallible;
use std::str;

use betterlite_core::api::realtime_backend_relay::raw::parse_backend_event;
use betterlite_core::api::realtime_contracts::config::{BaseRealtimeConfig, Environment};
use betterlite_core::api::realtime_contracts::error::HeaderMap;
use betterlite_core::api::realtime_contracts::events::{
    AudioTranscriptionUsage, RealtimeResponseTransformInput, RealtimeResponseTypedDict,
};
use betterlite_core::api::realtime_contracts::transport::RealtimeMessage;
use betterlite_core::api::realtime_session_state::RealtimeProviderConfig;
use betterlite_models::decisions::Json;

use super::transport::RecordingBackend;

// The provider echoes every backend object frame as the client event and reports whether it defers session setup
#[derive(Debug, Clone, Copy)]
pub(crate) struct ScriptedProvider {
    pub(crate) requires_session_configuration: bool,
}

impl BaseRealtimeConfig for ScriptedProvider {
    type Error = Infallible;
    type Backend = RecordingBackend;

    fn validate_environment(
        &self,
        _environment: &Environment<'_>,
    ) -> Result<HeaderMap, Infallible> {
        Ok(HeaderMap::new())
    }

    fn transform_realtime_request(
        &self,
        message: &str,
        _model: &str,
        _session_configuration_request: Option<&str>,
    ) -> Result<Vec<RealtimeMessage>, Infallible> {
        Ok(vec![RealtimeMessage::Text(message.to_owned())])
    }

    fn transform_realtime_response(
        &self,
        message: &RealtimeMessage,
        _model: &str,
        _input: &RealtimeResponseTransformInput,
    ) -> Result<RealtimeResponseTypedDict, Infallible> {
        let text = match message {
            RealtimeMessage::Text(text) => text.as_str(),
            RealtimeMessage::Binary(bytes) => str::from_utf8(bytes).unwrap_or_default(),
        };
        Ok(RealtimeResponseTypedDict {
            response: parse_backend_event(text).unwrap_or(Json::Null),
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

impl RealtimeProviderConfig for ScriptedProvider {
    fn requires_session_configuration(&self) -> bool {
        self.requires_session_configuration
    }

    fn unbilled_usage_on_close(&self, _model: &str) -> Option<AudioTranscriptionUsage> {
        None
    }
}
