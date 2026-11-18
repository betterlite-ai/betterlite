use super::backend::{FrameKind, classify_text};
use super::client::event_type_label;
use super::enforce::enforce_session_update;
use super::error::RealtimeSenderError;
use super::{RealtimeSender, SenderFuture};
use crate::api::realtime_beta_protocol::events::translate_event_to_beta;
use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_contracts::transport::RealtimeMessage;
use crate::api::realtime_guardrail_policy::{
    TRANSCRIPTION_EVENT_HOOKS, disable_auto_response_message, guardrails_run_for_hooks,
    inject_auto_response_disable,
};
use betterlite_models::decisions::Json;
use futures_util::FutureExt as _;
use std::error::Error;

impl<Config, CallbackError> RealtimeSender<'_, Config, CallbackError>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + 'static,
{
    // LiteLLM enforces the authorized model, transforms through the provider, and records state only after a frame is sent
    #[must_use]
    pub fn send_to_backend<'call>(
        &'call mut self,
        message: &'call str,
    ) -> SenderFuture<'call, bool, Config::Error, CallbackError> {
        async move {
            let enforced = self.enforce_session_model(message)?;
            let Some(provider) = self.provider else {
                self.send_frame(RealtimeMessage::Text(enforced)).await?;
                return Ok(true);
            };
            let transformed = provider
                .transform_realtime_request(
                    &enforced,
                    &self.state.model,
                    self.state.response.session_configuration_request.as_deref(),
                )
                .map_err(|source| RealtimeSenderError::Transform { source })?;
            let mut sent = false;
            for frame in transformed {
                sent |= self.deliver_frame(provider, frame).await?;
            }
            Ok(sent)
        }
        .boxed_local()
    }

    fn deliver_frame<'call>(
        &'call mut self,
        provider: &'call Config,
        frame: RealtimeMessage,
    ) -> SenderFuture<'call, bool, Config::Error, CallbackError> {
        match frame {
            RealtimeMessage::Binary(payload) => async move {
                provider.pace_backend_send(&payload).await;
                self.send_frame(RealtimeMessage::Binary(payload)).await?;
                self.state.setup.content_sent_after_setup.0 = true;
                Ok(true)
            }
            .boxed_local(),
            RealtimeMessage::Text(text) => {
                let kind = classify_text(provider, &text);
                self.deliver_text(text, kind)
            }
        }
    }

    fn send_frame(
        &mut self,
        frame: RealtimeMessage,
    ) -> SenderFuture<'_, (), Config::Error, CallbackError> {
        async move {
            self.state
                .backend
                .as_mut()
                .ok_or(RealtimeSenderError::BackendMissing)?
                .send(frame)
                .await
                .map_err(|source| RealtimeSenderError::Backend { source })
        }
        .boxed_local()
    }

    fn deliver_text(
        &mut self,
        mut text: String,
        kind: FrameKind,
    ) -> SenderFuture<'_, bool, Config::Error, CallbackError> {
        async move {
            if kind == FrameKind::Setup {
                if self.state.setup.content_sent_after_setup.0 {
                    tracing::debug!(
                        "Dropping follow-up setup after content was already sent to backend"
                    );
                    return Ok(false);
                }
                // Setup injection commits its flag before sending, unlike the setup cache below
                let injected = inject_auto_response_disable(
                    &text,
                    &mut self.state.setup.turn_detection_update_sent.0,
                    self.guardrails,
                    &self.state.request_data,
                )
                .map_err(|source| RealtimeSenderError::GuardrailSetup { source })?;
                text = injected.unwrap_or(text);
            }
            self.send_frame(RealtimeMessage::Text(text.clone())).await?;
            self.cache_session_configuration(&text);
            if kind == FrameKind::Content {
                self.state.setup.content_sent_after_setup.0 = true;
            }
            Ok(true)
        }
        .boxed_local()
    }

    // A provider normalizer that rejects an event keeps it off the client socket
    #[must_use]
    pub fn should_drop_event_from_client(&self, event: &Json) -> bool {
        let Some(normalizer) = self.state.event_normalizer.as_deref() else {
            return false;
        };
        normalizer.should_drop(event)
    }

    #[must_use]
    pub fn normalize_event_for_ga_client(&self, event: Json) -> Json {
        let Some(normalizer) = self.state.event_normalizer.as_deref() else {
            return event;
        };
        normalizer.normalize(event)
    }

    pub fn event_to_client_json(
        &self,
        event: Json,
    ) -> Result<String, RealtimeSenderError<Config::Error, CallbackError>> {
        serde_json::to_string(&self.normalize_event_for_ga_client(event))
            .map_err(|source| RealtimeSenderError::Encode { source })
    }

    // LiteLLM normalizes a provider event, translates it for a beta client, and falls back to the untranslated frame
    #[must_use]
    pub fn send_event_to_client<'call>(
        &'call mut self,
        event: &'call Json,
        event_str: &'call str,
    ) -> SenderFuture<'call, bool, Config::Error, CallbackError> {
        async move {
            if self.should_drop_event_from_client(event) {
                return Ok(false);
            }
            let mut normalized = None;
            let mut encoded = event_str.to_owned();
            if matches!(event, Json::Object(_)) {
                let value = self.normalize_event_for_ga_client(event.clone());
                encoded = serde_json::to_string(&value)
                    .map_err(|source| RealtimeSenderError::Encode { source })?;
                normalized = Some(value);
            }
            if let Some(object) = &normalized
                && self.state.client_wants_beta.0
            {
                match self.send_translated(object).await {
                    Ok(delivered) => return Ok(delivered),
                    Err(error) => tracing::warn!(
                        "Failed to translate {} to beta protocol, forwarding untranslated event to client: {error}",
                        event_type_label(object)
                    ),
                }
            }
            self.send_client_text(&encoded).await?;
            Ok(true)
        }
        .boxed_local()
    }

    fn send_translated<'call>(
        &'call mut self,
        event: &'call Json,
    ) -> SenderFuture<'call, bool, Config::Error, CallbackError> {
        async move {
            let outcome = translate_event_to_beta(event.clone())
                .map_err(|source| RealtimeSenderError::Beta { source })?;
            let Some(translated) = outcome else {
                return Ok(false);
            };
            let encoded = serde_json::to_string(&translated)
                .map_err(|source| RealtimeSenderError::Encode { source })?;
            self.send_client_text(&encoded).await?;
            Ok(true)
        }
        .boxed_local()
    }

    fn send_client_text<'call>(
        &'call mut self,
        text: &'call str,
    ) -> SenderFuture<'call, (), Config::Error, CallbackError> {
        async move {
            self.state
                .client
                .as_mut()
                .ok_or(RealtimeSenderError::ClientMissing)?
                .send_text(text)
                .await
                .map_err(|source| RealtimeSenderError::Client { source })
        }
        .boxed_local()
    }

    // LiteLLM caches the exact frame it sent whenever the sent JSON object carries a setup key, even a null one
    pub fn cache_session_configuration(&mut self, message: &str) {
        let Ok(Json::Object(root)) = serde_json::from_str::<Json>(message) else {
            return;
        };
        if root.contains_key("setup") {
            self.state.response.session_configuration_request = Some(message.to_owned());
        }
    }

    // LiteLLM retries the VAD disable on the next opportunity until the provider transformation actually delivers it
    #[must_use]
    pub fn maybe_disable_auto_response(
        &mut self,
    ) -> SenderFuture<'_, (), Config::Error, CallbackError> {
        async move {
            if self.state.setup.turn_detection_update_sent.0 || !self.transcription_guardrails()? {
                return Ok(());
            }
            let message = disable_auto_response_message(self.state.backend_uses_beta_protocol.0)
                .map_err(|source| RealtimeSenderError::Disable { source })?;
            if self.send_to_backend(&message).await? {
                self.state.setup.turn_detection_update_sent.0 = true;
            }
            Ok(())
        }
        .boxed_local()
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

    pub fn enforce_session_model(
        &mut self,
        message: &str,
    ) -> Result<String, RealtimeSenderError<Config::Error, CallbackError>> {
        let Some(authorized_model) = self.state.force_transcription_model.clone() else {
            return Ok(message.to_owned());
        };
        enforce_session_update(
            message,
            &authorized_model,
            &mut self.state.is_transcription_session.0,
        )
    }
}
