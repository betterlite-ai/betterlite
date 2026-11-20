pub(super) mod events;
pub(super) mod guardrails;
pub(super) mod send;

use std::error::Error;

use async_trait::async_trait;
use betterlite_models::decisions::Json;

use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_contracts::events::RealtimeResponseTransformInput;
use crate::api::realtime_contracts::events::RealtimeResponseTypedDict;
use crate::api::realtime_contracts::transport::RealtimeMessage;

use self::events::provider_event;
use super::RealtimeBackendRelay;
use super::error::RealtimeRelayError;

#[async_trait]
pub trait ProviderConfigRelay<Config, CallbackError>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    async fn handle_provider_config_message(
        &mut self,
        provider: &Config,
        raw_response: &str,
    ) -> Result<(), RealtimeRelayError<Config::Error, CallbackError>>;
}

#[async_trait]
impl<Config, CallbackError> ProviderConfigRelay<Config, CallbackError>
    for RealtimeBackendRelay<'_, Config, CallbackError>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    async fn handle_provider_config_message(
        &mut self,
        provider: &Config,
        raw_response: &str,
    ) -> Result<(), RealtimeRelayError<Config::Error, CallbackError>> {
        let returned = provider
            .transform_realtime_response(
                &RealtimeMessage::Text(raw_response.to_owned()),
                &self.state.model,
                &RealtimeResponseTransformInput {
                    session_configuration_request: self
                        .state
                        .response
                        .session_configuration_request
                        .clone(),
                    current_output_item_id: self.state.response.current_output_item_id.clone(),
                    current_response_id: self.state.response.current_response_id.clone(),
                    current_delta_chunks: self.state.response.current_delta_chunks.clone(),
                    current_item_chunks: self.state.response.current_item_chunks.clone(),
                    current_conversation_id: self.state.response.current_conversation_id.clone(),
                    current_delta_type: self.state.response.current_delta_type,
                },
            )
            .map_err(RealtimeRelayError::Transform)?;
        let RealtimeResponseTypedDict {
            response,
            current_output_item_id,
            current_response_id,
            current_delta_chunks,
            current_conversation_id,
            current_item_chunks,
            current_delta_type,
            session_configuration_request,
        } = returned;
        self.state.response.current_output_item_id = current_output_item_id;
        self.state.response.current_response_id = current_response_id;
        self.state.response.current_delta_chunks = current_delta_chunks;
        self.state.response.current_conversation_id = current_conversation_id;
        self.state.response.current_item_chunks = current_item_chunks;
        self.state.response.current_delta_type = current_delta_type;
        self.state.response.session_configuration_request = session_configuration_request;
        let events = match response {
            Json::List(events) => events,
            event @ (Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::Object(_)) => {
                vec![event]
            }
        };
        for event in events {
            provider_event(self, event).await?;
        }
        Ok(())
    }
}
