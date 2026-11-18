use betterlite_models::decisions::Json;

use super::RealtimeSender;
use super::SenderFuture;
use super::error::RealtimeSenderError;
use crate::api::realtime_beta_protocol::events::translate_event_to_beta;
use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_contracts::transport::ClientWebSocket;

impl<Config, CallbackError> RealtimeSender<'_, Config, CallbackError>
where
    Config: BaseRealtimeConfig,
    CallbackError: std::error::Error + 'static,
{
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
    pub fn send_event_to_client(
        &mut self,
        event: &Json,
        event_str: &str,
    ) -> SenderFuture<'_, Result<bool, RealtimeSenderError<Config::Error, CallbackError>>> {
        Box::pin(async move {
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
            self.client()?
                .send_text(&encoded)
                .await
                .map_err(|source| RealtimeSenderError::Client { source })?;
            Ok(true)
        })
    }

    fn send_translated(
        &mut self,
        event: &Json,
    ) -> SenderFuture<'_, Result<bool, RealtimeSenderError<Config::Error, CallbackError>>> {
        Box::pin(async move {
            let outcome = translate_event_to_beta(event.clone())
                .map_err(|source| RealtimeSenderError::Beta { source })?;
            let Some(translated) = outcome else {
                return Ok(false);
            };
            let encoded = serde_json::to_string(&translated)
                .map_err(|source| RealtimeSenderError::Encode { source })?;
            self.client()?
                .send_text(&encoded)
                .await
                .map_err(|source| RealtimeSenderError::Client { source })?;
            Ok(true)
        })
    }

    fn client(
        &mut self,
    ) -> Result<&mut dyn ClientWebSocket, RealtimeSenderError<Config::Error, CallbackError>> {
        match self.state.client.as_deref_mut() {
            Some(client) => Ok(client),
            None => Err(RealtimeSenderError::ClientMissing),
        }
    }
}

pub(super) fn event_type_label(event: &Json) -> String {
    let Json::Object(fields) = event else {
        return String::from("<non-string>");
    };
    match fields.get("type") {
        Some(Json::Text(kind)) => kind.clone(),
        _ => String::from("<non-string>"),
    }
}
