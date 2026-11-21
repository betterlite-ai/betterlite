use std::error::Error as StdError;
use std::fmt::{self, Display, Formatter};

use async_trait::async_trait;
use betterlite_core::api::realtime_contracts::config::{BaseRealtimeConfig, Environment};
use betterlite_core::api::realtime_contracts::error::{HeaderMap, WebSocketError};
use betterlite_core::api::realtime_contracts::events::{
    RealtimeResponseTransformInput, RealtimeResponseTypedDict,
};
use betterlite_core::api::realtime_contracts::transport::{RealtimeBackend, RealtimeMessage};
use betterlite_models::decisions::Json;

use super::input::{ProviderDocument, ProviderKind};

// The provider's own failure, carrying the Python class name the helper reports so both sides raise the same error
#[derive(Debug)]
pub(crate) struct ScriptedError {
    pub(crate) class: String,
    pub(crate) message: String,
}

impl Display for ScriptedError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl StdError for ScriptedError {}

pub(crate) struct UnusedBackend;

#[async_trait]
impl RealtimeBackend for UnusedBackend {
    async fn send(&mut self, _message: RealtimeMessage) -> Result<(), WebSocketError> {
        Err(WebSocketError {
            message: String::from("the scripted provider opens no backend"),
        })
    }

    async fn recv(&mut self, _decode: Option<bool>) -> Result<RealtimeMessage, WebSocketError> {
        Err(WebSocketError {
            message: String::from("the scripted provider opens no backend"),
        })
    }

    async fn close(&mut self) -> Result<(), WebSocketError> {
        Ok(())
    }
}

// A provider scripted by the case: Gemini's setup and synthetic event when they are wanted, and one exact validate failure
pub(crate) struct ScriptedProvider {
    pub(crate) document: ProviderDocument,
    pub(crate) url: String,
}

#[async_trait]
impl BaseRealtimeConfig for ScriptedProvider {
    type Error = ScriptedError;
    type Backend = UnusedBackend;

    fn validate_environment(
        &self,
        environment: &Environment<'_>,
    ) -> Result<HeaderMap, ScriptedError> {
        match &self.document.error {
            Some(error) => Err(ScriptedError {
                class: error.error_class.clone(),
                message: error.message.clone(),
            }),
            None => Ok(environment.headers.clone()),
        }
    }

    fn get_complete_url(
        &self,
        _api_base: Option<&str>,
        _model: &str,
        _api_key: Option<&str>,
    ) -> String {
        self.url.clone()
    }

    fn transform_realtime_request(
        &self,
        _message: &str,
        _model: &str,
        _session_configuration_request: Option<&str>,
    ) -> Result<Vec<RealtimeMessage>, ScriptedError> {
        Ok(Vec::new())
    }

    fn requires_session_configuration(&self) -> bool {
        matches!(self.document.kind, ProviderKind::Setup)
    }

    fn session_configuration_request(&self, _model: &str) -> Option<String> {
        match self.document.kind {
            ProviderKind::Setup => self.document.setup.clone(),
            ProviderKind::Deferred | ProviderKind::Error => None,
        }
    }

    async fn open_backend(
        &self,
        _url: &str,
        _headers: &HeaderMap,
    ) -> Result<Option<Self::Backend>, ScriptedError> {
        Ok(None)
    }

    fn session_created_event(
        &self,
        _model: &str,
        _logging_session_id: &str,
        _session_configuration_request: Option<&str>,
    ) -> Option<Json> {
        match self.document.kind {
            ProviderKind::Deferred => self.document.synthetic.clone(),
            ProviderKind::Setup | ProviderKind::Error => None,
        }
    }

    fn transform_realtime_response(
        &self,
        _message: &RealtimeMessage,
        _model: &str,
        _input: &RealtimeResponseTransformInput,
    ) -> Result<RealtimeResponseTypedDict, ScriptedError> {
        Ok(RealtimeResponseTypedDict {
            response: Json::Null,
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
