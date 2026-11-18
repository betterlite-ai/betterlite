use super::backend::{FrameKind, classify_text, send_backend};
use super::client::{ClientFrame, event_type_label, send_client_frame};
use super::enforce::enforce_session_update;
use super::error::RealtimeSenderError;
use super::{RealtimeSend, RealtimeSender, SenderFuture};
use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_contracts::transport::RealtimeMessage;
use crate::api::realtime_guardrail_policy::{
    TRANSCRIPTION_EVENT_HOOKS, disable_auto_response_message, guardrails_run_for_hooks,
    inject_auto_response_disable,
};
use async_trait::async_trait;
use betterlite_models::decisions::Json;
use futures_util::FutureExt as _;
use std::error::Error;

impl<Config, CallbackError> RealtimeSender<'_, Config, CallbackError>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + 'static,
{
    fn deliver_frame<'call>(
        &'call mut self,
        provider: &'call Config,
        frame: RealtimeMessage,
    ) -> SenderFuture<'call, bool, Config::Error, CallbackError> {
        match frame {
            RealtimeMessage::Binary(payload) => async move {
                provider.pace_backend_send(&payload).await;
                send_backend::<Config::Error, CallbackError>(
                    &mut self.state.backend,
                    RealtimeMessage::Binary(payload),
                )
                .await?;
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

    fn deliver_text(
        &mut self,
        mut text: String,
        kind: FrameKind,
    ) -> SenderFuture<'_, bool, Config::Error, CallbackError> {
        async move {
            if kind == FrameKind::Setup && self.state.setup.content_sent_after_setup.0 {
                tracing::debug!(
                    "Dropping follow-up setup after content was already sent to backend"
                );
                return Ok(false);
            }
            if kind == FrameKind::Setup {
                // Setup injection commits its flag before sending, unlike the setup cache below
                let injected = inject_auto_response_disable(
                    &text,
                    &mut self.state.setup.turn_detection_update_sent.0,
                    self.guardrails,
                    &self.state.request_data,
                )
                .map_err(RealtimeSenderError::GuardrailSetup)?;
                text = injected.unwrap_or(text);
            }
            send_backend::<Config::Error, CallbackError>(
                &mut self.state.backend,
                RealtimeMessage::Text(text.clone()),
            )
            .await?;
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
            .map_err(RealtimeSenderError::Encode)
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

    fn transcription_guardrails(
        &self,
    ) -> Result<bool, RealtimeSenderError<Config::Error, CallbackError>> {
        guardrails_run_for_hooks(
            self.guardrails,
            &self.state.request_data,
            &TRANSCRIPTION_EVENT_HOOKS,
        )
        .map_err(RealtimeSenderError::Predicate)
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

#[async_trait(?Send)]
impl<Config, CallbackError> RealtimeSend for RealtimeSender<'_, Config, CallbackError>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + 'static,
{
    type Error = RealtimeSenderError<Config::Error, CallbackError>;

    async fn send_to_backend(&mut self, message: &str) -> Result<bool, Self::Error> {
        let enforced = self.enforce_session_model(message)?;
        let Some(provider) = self.provider else {
            send_backend::<Config::Error, CallbackError>(
                &mut self.state.backend,
                RealtimeMessage::Text(enforced),
            )
            .await?;
            return Ok(true);
        };
        let transformed = provider
            .transform_realtime_request(
                &enforced,
                &self.state.model,
                self.state.response.session_configuration_request.as_deref(),
            )
            .map_err(RealtimeSenderError::Transform)?;
        let mut sent = false;
        for frame in transformed {
            sent |= self.deliver_frame(provider, frame).await?;
        }
        Ok(sent)
    }

    async fn send_event_to_client(
        &mut self,
        event: &Json,
        event_str: &str,
    ) -> Result<bool, Self::Error> {
        if self.should_drop_event_from_client(event) {
            return Ok(false);
        }
        let mut normalized = None;
        let mut encoded = event_str.to_owned();
        if matches!(event, Json::Object(_)) {
            let value = self.normalize_event_for_ga_client(event.clone());
            encoded = serde_json::to_string(&value).map_err(RealtimeSenderError::Encode)?;
            normalized = Some(value);
        }
        if let Some(object) = &normalized
            && self.state.client_wants_beta.0
        {
            match send_client_frame::<Config::Error, CallbackError>(
                &mut self.state.client,
                ClientFrame::Beta(object),
            )
            .await
            {
                Ok(delivered) => return Ok(delivered),
                Err(error) => tracing::warn!(
                    "Failed to translate {} to beta protocol, forwarding untranslated event to client: {error}",
                    event_type_label(object)
                ),
            }
        }
        send_client_frame(&mut self.state.client, ClientFrame::Text(&encoded)).await
    }

    async fn maybe_disable_auto_response(&mut self) -> Result<(), Self::Error> {
        if self.state.setup.turn_detection_update_sent.0 || !self.transcription_guardrails()? {
            return Ok(());
        }
        let message = disable_auto_response_message(self.state.backend_uses_beta_protocol.0)
            .map_err(RealtimeSenderError::Disable)?;
        if self.send_to_backend(&message).await? {
            self.state.setup.turn_detection_update_sent.0 = true;
        }
        Ok(())
    }
}
