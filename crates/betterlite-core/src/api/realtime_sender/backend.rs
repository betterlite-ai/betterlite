use betterlite_models::decisions::Json;

use super::RealtimeSender;
use super::SenderFuture;
use super::error::RealtimeSenderError;
use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_contracts::transport::{RealtimeBackend, RealtimeMessage};

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
    pub fn send_to_backend(
        &mut self,
        message: &str,
    ) -> SenderFuture<'_, Result<bool, RealtimeSenderError<Config::Error, CallbackError>>> {
        Box::pin(async move {
            let enforced = self.enforce_session_model(message)?;
            let Some(provider) = self.provider else {
                self.send_direct(enforced).await?;
                return Ok(true);
            };
            let transformed = self.transform(provider, &enforced)?;
            let mut sent = false;
            for frame in transformed {
                let delivered = match frame {
                    RealtimeMessage::Binary(payload) => {
                        provider.pace_backend_send(&payload).await;
                        self.deliver_binary(payload).await?
                    }
                    RealtimeMessage::Text(text) => {
                        let kind = classify_text(provider, &text);
                        self.deliver_text(text, kind).await?
                    }
                };
                if delivered {
                    sent = true;
                }
            }
            Ok(sent)
        })
    }

    fn transform(
        &self,
        provider: &Config,
        message: &str,
    ) -> Result<Vec<RealtimeMessage>, RealtimeSenderError<Config::Error, CallbackError>> {
        provider
            .transform_realtime_request(
                message,
                &self.state.model,
                self.state.response.session_configuration_request.as_deref(),
            )
            .map_err(|source| RealtimeSenderError::Transform { source })
    }

    // Without a provider the enforced frame is what the backend gets, byte for byte
    fn send_direct(
        &mut self,
        message: String,
    ) -> SenderFuture<'_, Result<(), RealtimeSenderError<Config::Error, CallbackError>>> {
        Box::pin(async move {
            self.backend()?
                .send(RealtimeMessage::Text(message))
                .await
                .map_err(|source| RealtimeSenderError::Backend { source })
        })
    }

    fn deliver_binary(
        &mut self,
        payload: Vec<u8>,
    ) -> SenderFuture<'_, Result<bool, RealtimeSenderError<Config::Error, CallbackError>>> {
        Box::pin(async move {
            self.backend()?
                .send(RealtimeMessage::Binary(payload))
                .await
                .map_err(|source| RealtimeSenderError::Backend { source })?;
            self.state.setup.content_sent_after_setup.0 = true;
            Ok(true)
        })
    }

    fn deliver_text(
        &mut self,
        text: String,
        kind: FrameKind,
    ) -> SenderFuture<'_, Result<bool, RealtimeSenderError<Config::Error, CallbackError>>> {
        Box::pin(async move {
            if kind == FrameKind::Setup {
                if self.state.setup.content_sent_after_setup.0 {
                    tracing::debug!(
                        "Dropping follow-up setup after content was already sent to backend"
                    );
                    return Ok(false);
                }
                let message = self.fold_auto_response_disable(&text)?;
                self.backend()?
                    .send(RealtimeMessage::Text(message.clone()))
                    .await
                    .map_err(|source| RealtimeSenderError::Backend { source })?;
                self.cache_session_configuration(&message);
                return Ok(true);
            }
            self.backend()?
                .send(RealtimeMessage::Text(text.clone()))
                .await
                .map_err(|source| RealtimeSenderError::Backend { source })?;
            self.cache_session_configuration(&text);
            if kind == FrameKind::Content {
                self.state.setup.content_sent_after_setup.0 = true;
            }
            Ok(true)
        })
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
