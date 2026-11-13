use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthState {
    Healthy,
    Unhealthy,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct IntegrationHealthStatus {
    pub status: HealthState,
    pub error_message: Option<String>,
}

impl IntegrationHealthStatus {
    #[must_use]
    pub const fn healthy() -> Self {
        Self {
            status: HealthState::Healthy,
            error_message: None,
        }
    }

    #[must_use]
    pub const fn unhealthy(message: String) -> Self {
        Self {
            status: HealthState::Unhealthy,
            error_message: Some(message),
        }
    }
}
