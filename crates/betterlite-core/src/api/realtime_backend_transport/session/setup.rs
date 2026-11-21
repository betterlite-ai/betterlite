use std::error::Error as StdError;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::realtime_backend_transport::error::RealtimeConnectionError;
use crate::api::realtime_backend_transport::protocol::RealtimeSessionRelay;
use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_contracts::error::{RealtimeContractError, WebSocketError};
use crate::api::realtime_contracts::transport::{RealtimeBackend, RealtimeMessage};
use crate::api::realtime_guardrail_policy::{
    RealtimeGuardrailCallback, inject_auto_response_disable,
};
use crate::api::realtime_session_state::RealtimeStreaming;
use crate::api::realtime_session_state::store::SessionMessage;

use super::RealtimeCall;

impl<Config, Callback, Relay> RealtimeCall<'_, Config, Callback, Relay>
where
    Config: BaseRealtimeConfig + 'static,
    Callback: RealtimeGuardrailCallback + ?Sized,
    Callback::Error: StdError + Send + 'static,
    Relay: RealtimeSessionRelay,
{
    // The one frame a provider opens with: its setup plus the guardrail's disable, or the synthetic session.created that unblocks a deferred client
    pub(super) async fn send_initial_frame(
        &self,
        state: &mut RealtimeStreaming,
    ) -> Result<(), RealtimeConnectionError<Config::Error, Callback::Error, Relay::Error>> {
        if self.provider.requires_session_configuration() {
            let Some(setup) = self.session_setup(state)? else {
                return Ok(());
            };
            send_to_backend(state, &setup).await?;
            state.response.session_configuration_request = Some(setup);
            return Ok(());
        }
        let Some(event) =
            self.provider
                .session_created_event(self.model, self.logging_session_id, None)
        else {
            return Ok(());
        };
        self.deliver_synthetic(state, &event).await
    }

    // The provider's setup with the guardrail's activity-detection disable injected; a missing or empty setup means none is sent
    fn session_setup(
        &self,
        state: &mut RealtimeStreaming,
    ) -> Result<Option<String>, RealtimeConnectionError<Config::Error, Callback::Error, Relay::Error>>
    {
        let Some(request) = self
            .provider
            .session_configuration_request(self.model)
            .filter(|request| !request.is_empty())
        else {
            return Ok(None);
        };
        let patched = inject_auto_response_disable(
            &request,
            &mut state.setup.turn_detection_update_sent.0,
            self.callbacks,
            &state.request_data,
        )
        .map_err(RealtimeConnectionError::Guardrail)?;
        Ok(Some(patched.unwrap_or(request)))
    }

    // A deferred provider's synthetic session.created is stored before the client send and flagged only after it
    async fn deliver_synthetic(
        &self,
        state: &mut RealtimeStreaming,
        event: &Json,
    ) -> Result<(), RealtimeConnectionError<Config::Error, Callback::Error, Relay::Error>> {
        let text = serde_json::to_string(event).map_err(|source| {
            RealtimeConnectionError::Contract(RealtimeContractError::Json(source))
        })?;
        state
            .store_message(SessionMessage::Text(&text))
            .map_err(RealtimeConnectionError::Session)?;
        self.client
            .socket
            .lock()
            .await
            .send_text(&text)
            .await
            .map_err(RealtimeConnectionError::Client)?;
        state.setup.session_created_sent_to_client.0 = true;
        tracing::debug!("Sent synthetic session.created to client to unblock connection");
        Ok(())
    }
}

async fn send_to_backend<ConfigError, CallbackError, RelayError>(
    state: &mut RealtimeStreaming,
    text: &str,
) -> Result<(), RealtimeConnectionError<ConfigError, CallbackError, RelayError>>
where
    ConfigError: StdError + 'static,
    CallbackError: StdError + 'static,
    RelayError: StdError + 'static,
{
    let Some(backend) = state.backend.as_mut() else {
        return Err(RealtimeConnectionError::Backend(WebSocketError {
            message: "the realtime backend websocket is not connected".to_owned(),
        }));
    };
    backend
        .send(RealtimeMessage::Text(text.to_owned()))
        .await
        .map_err(RealtimeConnectionError::Backend)
}

// LiteLLM reads intent only to force the authorized model on a transcription session; the query is never appended to the provider URL
#[must_use]
pub(super) fn forced_transcription_model(
    model: &str,
    query_params: Option<&IndexMap<String, String>>,
) -> Option<String> {
    match query_params.and_then(|params| params.get("intent").map(String::as_str)) {
        Some("transcription") => Some(model.to_owned()),
        Some(_other) => None,
        None => None,
    }
}
