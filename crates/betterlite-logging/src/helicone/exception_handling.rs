use std::str;

use betterlite_http::proxy::UpstreamResponse;
use thiserror::Error;

use super::envelope::{self, EnvelopeError};
use super::event::SuccessEvent;
use super::http_delivery::{self, DeliveryError};
use super::initialization::HeliconeLogger;

#[derive(Debug, Error)]
pub enum LogError {
    #[error("HELICONE_API_KEY is not set")]
    MissingKey,
    #[error("the Helicone payload could not be built")]
    Envelope(#[from] EnvelopeError),
    #[error("the Helicone payload could not be delivered")]
    Delivery(#[from] DeliveryError),
}

impl HeliconeLogger {
    // Nothing escapes this method: a failure only becomes a debug line, as LiteLLM's bare except does.
    pub async fn log_success(&self, event: &SuccessEvent) {
        tracing::debug!(
            model = %event.model,
            "Helicone Logging - Enters logging function for model"
        );
        match self.deliver(event).await {
            Ok(response) if response.status == 200 => {
                tracing::debug!("Helicone Logging - Success!");
            }
            Ok(response) => {
                tracing::debug!(
                    status = response.status,
                    body = str::from_utf8(&response.body).unwrap_or("<not UTF-8>"),
                    "Helicone Logging - Error Request was not successful"
                );
            }
            Err(error) => tracing::debug!(?error, "Helicone Logging Error"),
        }
    }

    async fn deliver(&self, event: &SuccessEvent) -> Result<UpstreamResponse, LogError> {
        let key = self.settings.api_key.as_ref().ok_or(LogError::MissingKey)?;
        let assembled = envelope::assemble(&self.settings, key, event)?;
        Ok(http_delivery::post(&self.client, key, assembled, self.timeout).await?)
    }
}

mod tests;
