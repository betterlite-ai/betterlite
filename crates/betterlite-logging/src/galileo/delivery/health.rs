use std::sync::PoisonError;

use serde::Serialize;

use super::GalileoDelivery;
use super::auth::ensure_headers;
use super::error::DeliveryError;
use super::transport::Transport;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthState {
    Healthy,
    Unhealthy,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HealthStatus {
    pub status: HealthState,
    pub error_message: Option<String>,
}

// GalileoObserve.async_health_check.
pub fn check(delivery: &GalileoDelivery, transport: &dyn Transport) -> HealthStatus {
    match probe(delivery, transport) {
        Ok(()) => HealthStatus {
            status: HealthState::Healthy,
            error_message: None,
        },
        Err(failure) => HealthStatus {
            status: HealthState::Unhealthy,
            error_message: Some(failure.to_string()),
        },
    }
}

fn probe(delivery: &GalileoDelivery, transport: &dyn Transport) -> Result<(), DeliveryError> {
    let config = &delivery.config;
    if config.project_id.is_none() {
        return Err(DeliveryError::MissingProjectId);
    }
    let Some(base_url) = &config.base_url else {
        return Err(DeliveryError::MissingBaseUrl);
    };
    if !config.use_v2_api && (config.username.is_none() || config.password.is_none()) {
        return Err(DeliveryError::MissingCredentials);
    }
    if !ensure_headers(delivery, transport) {
        return Err(DeliveryError::AuthenticationFailed);
    }
    let headers = delivery
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .headers
        .clone()
        .unwrap_or_default();
    let response = transport
        .get(&format!("{base_url}/current_user"), &headers)
        .map_err(DeliveryError::ProbeFailed)?;
    if response.status >= 400 {
        return Err(DeliveryError::ProbeStatus {
            status: response.status,
        });
    }
    Ok(())
}
