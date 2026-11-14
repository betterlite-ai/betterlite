use thiserror::Error;

use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_records::error::GuardrailRecordError;
use crate::headroom_messages::HeadroomMessagesError;
use crate::headroom_settings::HeadroomSettingsError;

#[derive(Debug, Error)]
pub enum HeadroomError {
    #[error("the Headroom HTTP client could not be built")]
    Client {
        #[source]
        source: reqwest::Error,
    },
    #[error(transparent)]
    Settings(#[from] HeadroomSettingsError),
    #[error(transparent)]
    Messages(#[from] HeadroomMessagesError),
    #[error(transparent)]
    Record(#[from] GuardrailRecordError),
    #[error("a Headroom message value could not be read as JSON")]
    Json {
        #[from]
        source: serde_json::Error,
    },
}

impl From<HeadroomError> for GuardrailFailure {
    fn from(error: HeadroomError) -> Self {
        Self::Other(error.to_string())
    }
}
