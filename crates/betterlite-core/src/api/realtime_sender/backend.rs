use betterlite_models::decisions::Json;

use super::RealtimeSender;
use super::SenderFuture;
use super::error::RealtimeSenderError;
use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_contracts::transport::{RealtimeBackend, RealtimeMessage};
use crate::api::realtime_guardrail_policy::inject_auto_response_disable;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FrameKind {
    Setup,
    Content,
    Plain,
}

impl<Config, CallbackError> RealtimeSender<'_, Config, CallbackError>
where
    Config: BaseRealtimeConfig,
    CallbackError: std::error::Error + 'static,
{
    // LiteLLM enforces the authorized model, transforms through the provider, and records state only after a frame is sent
    #[must_use]
    pub fn send_to_backend<'call>(
        &'call mut self,
        message: &'call str,
    ) -> SenderFuture<'call, Result<bool, RealtimeSenderError<Config::Error, CallbackError>>> {
        Box::pin(async move {
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
        })
    }

    async fn deliver_frame(
        &mut self,
        provider: &Config,
        frame: RealtimeMessage,
    ) -> Result<bool, RealtimeSenderError<Config::Error, CallbackError>> {
        match frame {
            RealtimeMessage::Binary(payload) => {
                provider.pace_backend_send(&payload).await;
                self.send_frame(RealtimeMessage::Binary(payload)).await?;
                self.state.setup.content_sent_after_setup.0 = true;
                Ok(true)
            }
            RealtimeMessage::Text(text) => {
                let kind = classify_text(provider, &text);
                self.deliver_text(text, kind).await
            }
        }
    }

    async fn send_frame(
        &mut self,
        frame: RealtimeMessage,
    ) -> Result<(), RealtimeSenderError<Config::Error, CallbackError>> {
        self.backend()?
            .send(frame)
            .await
            .map_err(|source| RealtimeSenderError::Backend { source })
    }

    async fn deliver_text(
        &mut self,
        text: String,
        kind: FrameKind,
    ) -> Result<bool, RealtimeSenderError<Config::Error, CallbackError>> {
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
            let message = injected.unwrap_or(text);
            self.send_frame(RealtimeMessage::Text(message.clone()))
                .await?;
            self.cache_session_configuration(&message);
            return Ok(true);
        }
        self.send_frame(RealtimeMessage::Text(text.clone())).await?;
        self.cache_session_configuration(&text);
        if kind == FrameKind::Content {
            self.state.setup.content_sent_after_setup.0 = true;
        }
        Ok(true)
    }

    fn backend(
        &mut self,
    ) -> Result<&mut dyn RealtimeBackend, RealtimeSenderError<Config::Error, CallbackError>> {
        match self.state.backend.as_deref_mut() {
            Some(backend) => Ok(backend),
            None => Err(RealtimeSenderError::BackendMissing),
        }
    }
}

fn classify_text<Config>(provider: &Config, text: &str) -> FrameKind
where
    Config: BaseRealtimeConfig,
{
    let decoded = serde_json::from_str::<Json>(text).ok();
    match &decoded {
        Some(value @ Json::Object(_)) => {
            if provider.is_setup_message(value) {
                FrameKind::Setup
            } else if provider.is_content_message(value) {
                FrameKind::Content
            } else {
                FrameKind::Plain
            }
        }
        _ => FrameKind::Plain,
    }
}
