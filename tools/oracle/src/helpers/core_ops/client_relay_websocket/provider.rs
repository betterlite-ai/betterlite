use std::convert::Infallible;

use betterlite_core::api::realtime_contracts::config::{BaseRealtimeConfig, Environment};
use betterlite_core::api::realtime_contracts::error::HeaderMap;
use betterlite_core::api::realtime_contracts::events::{
    RealtimeResponseTransformInput, RealtimeResponseTypedDict,
};
use betterlite_core::api::realtime_contracts::transport::RealtimeMessage;
use betterlite_models::decisions::Json;

use super::transport::ScriptedBackend;

// The relay is generic over its provider, and a scripted session never configures one
pub(crate) struct NoProvider;

impl BaseRealtimeConfig for NoProvider {
    type Error = Infallible;
    type Backend = ScriptedBackend;

    fn validate_environment(
        &self,
        _environment: &Environment<'_>,
    ) -> Result<HeaderMap, Infallible> {
        Ok(HeaderMap::new())
    }

    fn transform_realtime_request(
        &self,
        _message: &str,
        _model: &str,
        _session_configuration_request: Option<&str>,
    ) -> Result<Vec<RealtimeMessage>, Infallible> {
        Ok(Vec::new())
    }

    fn transform_realtime_response(
        &self,
        _message: &RealtimeMessage,
        _model: &str,
        _input: &RealtimeResponseTransformInput,
    ) -> Result<RealtimeResponseTypedDict, Infallible> {
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
