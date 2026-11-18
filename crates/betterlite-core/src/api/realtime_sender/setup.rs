use betterlite_models::decisions::Json;

use super::RealtimeSender;
use super::SenderFuture;
use super::error::RealtimeSenderError;
use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_guardrail_policy::{
    TRANSCRIPTION_EVENT_HOOKS, disable_auto_response_message, guardrails_run_for_hooks,
    inject_auto_response_disable,
};

impl<Config, CallbackError> RealtimeSender<'_, Config, CallbackError>
where
    Config: BaseRealtimeConfig,
    CallbackError: std::error::Error + 'static,
{
    // LiteLLM caches the exact frame it sent whenever the sent JSON object carries a setup key, even a null one
    pub fn cache_session_configuration(&mut self, message: &str) {
        let Ok(Json::Object(root)) = serde_json::from_str::<Json>(message) else {
            return;
        };
        if root.contains_key("setup") {
            self.state.response.session_configuration_request = Some(message.to_owned());
        }
    }

    // The guardrail flag flips before the setup send: a failed send still counts as the one folded disable
    pub(super) fn fold_auto_response_disable(
        &mut self,
        setup_message: &str,
    ) -> Result<String, RealtimeSenderError<Config::Error, CallbackError>> {
        inject_auto_response_disable(
            setup_message,
            &mut self.state.setup.turn_detection_update_sent.0,
            self.guardrails,
            &self.state.request_data,
        )
        .map(|injected| injected.unwrap_or_else(|| setup_message.to_owned()))
        .map_err(|source| RealtimeSenderError::GuardrailSetup { source })
    }

    // LiteLLM retries the VAD disable on the next opportunity until the provider transformation actually delivers it
    #[must_use]
    pub fn maybe_disable_auto_response(
        &mut self,
    ) -> SenderFuture<'_, Result<(), RealtimeSenderError<Config::Error, CallbackError>>> {
        Box::pin(async move {
            if self.state.setup.turn_detection_update_sent.0 {
                return Ok(());
            }
            if !self.transcription_guardrails()? {
                return Ok(());
            }
            let message = disable_auto_response_message(self.state.backend_uses_beta_protocol.0)
                .map_err(|source| RealtimeSenderError::Disable { source })?;
            if self.send_to_backend(&message).await? {
                self.state.setup.turn_detection_update_sent.0 = true;
            }
            Ok(())
        })
    }

    fn transcription_guardrails(
        &self,
    ) -> Result<bool, RealtimeSenderError<Config::Error, CallbackError>> {
        guardrails_run_for_hooks(
            self.guardrails,
            &self.state.request_data,
            &TRANSCRIPTION_EVENT_HOOKS,
        )
        .map_err(|source| RealtimeSenderError::Predicate { source })
    }
}
