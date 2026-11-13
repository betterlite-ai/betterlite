pub mod base_url;
pub mod credentials;
pub mod environment;
pub mod schema;
pub mod signing;

mod tests;

use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
#[error("{message}")]
pub struct OciError {
    pub status: u16,
    pub message: String,
}

impl From<OciError> for ProviderError {
    fn from(error: OciError) -> Self {
        Self::from(ProviderErrorParts {
            status_code: error.status,
            message: error.message,
            headers: None,
            request: None,
            response: None,
            body: None,
            status_code_is_synthesized: false,
        })
    }
}

pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "servingType", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OciServingMode {
    OnDemand {
        #[serde(rename = "modelId")]
        model_id: String,
    },
    Dedicated {
        #[serde(rename = "endpointId")]
        endpoint_id: String,
    },
}
