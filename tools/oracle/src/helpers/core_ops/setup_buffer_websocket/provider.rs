use async_trait::async_trait;
use betterlite_core::api::realtime_contracts::config::{BaseRealtimeConfig, Environment};
use betterlite_core::api::realtime_contracts::error::HeaderMap;
use betterlite_core::api::realtime_contracts::events::{
    AudioTranscriptionUsage, RealtimeResponseTransformInput, RealtimeResponseTypedDict,
};
use betterlite_core::api::realtime_contracts::transport::RealtimeMessage;
use betterlite_core::api::realtime_session_state::RealtimeProviderConfig;
use thiserror::Error;

use super::input::ProviderDocument;
use super::transport::WireBackend;

#[derive(Debug, Error)]
pub(crate) enum ScriptError {
    #[error("the scripted provider transformation failed")]
    Injected,
    #[error("the scripted setup buffer provider does not transform responses")]
    ResponseNotScripted,
}

pub(crate) struct ScriptedProvider {
    requires_session_configuration: bool,
    fail_transform: bool,
    dropped: Vec<String>,
}

impl ScriptedProvider {
    pub(crate) fn new(document: &ProviderDocument) -> Self {
        Self {
            requires_session_configuration: document.requires_session_configuration,
            fail_transform: document.fail_transform,
            dropped: document.dropped.clone(),
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
    ) -> Result<HeaderMap, ScriptError> {
        Ok(HeaderMap::new())
    }

    // Without a scripted batch the provider passes the frame through unchanged, so only the buffer decides what reaches the backend
    fn transform_realtime_request(
        &self,
        message: &str,
        _model: &str,
        _session_configuration_request: Option<&str>,
    ) -> Result<Vec<RealtimeMessage>, ScriptError> {
        if self.fail_transform {
            return Err(ScriptError::Injected);
        }
        if self.dropped.iter().any(|frame| frame == message) {
            return Ok(Vec::new());
        }
        Ok(vec![RealtimeMessage::Text(message.to_owned())])
    }

    fn requires_session_configuration(&self) -> bool {
        self.requires_session_configuration
    }

    fn transform_realtime_response(
        &self,
        _message: &RealtimeMessage,
        _model: &str,
        _input: &RealtimeResponseTransformInput,
    ) -> Result<RealtimeResponseTypedDict, ScriptError> {
        Err(ScriptError::ResponseNotScripted)
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
