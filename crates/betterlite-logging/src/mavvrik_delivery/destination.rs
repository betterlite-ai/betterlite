use std::sync::atomic::{AtomicBool, Ordering};

use secrecy::{ExposeSecret as _, SecretString};

use super::api::{
    API_ERROR_BODY_BYTES, MetricsMarker, RegisterRequest, RegisterResponse, reject_disconnected,
    send_api, signed_url, update_metrics_marker,
};
use super::error::MavvrikDeliveryError;
use super::gcs_upload::upload_to_gcs;
use super::transport::{Method, Transport, TransportRequest, TransportResponse};
use super::validation::{ValidationError, validate_api_endpoint};
use crate::focus_records::destinations::{FocusDestination, FocusTimeWindow};

#[derive(Clone, Debug, Default)]
pub struct MavvrikSettings {
    pub api_key: Option<SecretString>,
    pub api_endpoint: Option<String>,
    pub connection_id: Option<String>,
}

#[derive(Debug)]
pub struct FocusMavvrikDestination<Http: Transport> {
    pub(super) transport: Http,
    pub(super) api_key: SecretString,
    pub(super) connection_id: String,
    pub(super) agent_url: String,
    pub(super) upload_url_endpoint: String,
    pub(super) registered: AtomicBool,
}

impl<Http: Transport> FocusMavvrikDestination<Http> {
    pub fn new(transport: Http, settings: MavvrikSettings) -> Result<Self, ValidationError> {
        let api_key = settings
            .api_key
            .filter(|key| !key.expose_secret().is_empty())
            .ok_or(ValidationError::MissingSetting {
                key: "MAVVRIK_API_KEY",
            })?;
        let api_endpoint = settings
            .api_endpoint
            .filter(|endpoint| !endpoint.is_empty())
            .ok_or(ValidationError::MissingSetting {
                key: "MAVVRIK_API_ENDPOINT",
            })?;
        let connection_id = settings
            .connection_id
            .filter(|connection_id| !connection_id.is_empty())
            .ok_or(ValidationError::MissingSetting {
                key: "MAVVRIK_CONNECTION_ID",
            })?;
        validate_api_endpoint(&api_endpoint)?;
        let agent_url = format!(
            "{}/metrics/agent/ai/{connection_id}",
            api_endpoint.trim_end_matches('/')
        );
        Ok(Self {
            transport,
            api_key,
            upload_url_endpoint: format!("{agent_url}/upload-url"),
            agent_url,
            connection_id,
            registered: AtomicBool::new(false),
        })
    }

    pub(super) async fn send(
        &self,
        request: TransportRequest,
    ) -> Result<TransportResponse, MavvrikDeliveryError<Http::Error>> {
        self.transport
            .send(request)
            .await
            .map_err(|source| MavvrikDeliveryError::Transport { source })
    }

    // Always posts, unlike deliver(), which skips the call once this instance is registered
    pub async fn get_metrics_marker(
        &self,
    ) -> Result<MetricsMarker, MavvrikDeliveryError<Http::Error>> {
        let body = serde_json::to_vec(&RegisterRequest {
            name: &self.connection_id,
        })
        .map_err(MavvrikDeliveryError::Json)?;
        let response = send_api(self, Method::Post, self.agent_url.clone(), body).await?;
        reject_disconnected(self, &response)?;
        if response.status >= 400 {
            return Err(MavvrikDeliveryError::RegisterFailed {
                status: response.status,
                body: response.text_prefix(API_ERROR_BODY_BYTES),
            });
        }
        self.registered.store(true, Ordering::SeqCst);
        let parsed: RegisterResponse =
            serde_json::from_slice(&response.body).map_err(MavvrikDeliveryError::Json)?;
        tracing::debug!(
            metrics_marker = ?parsed.metrics_marker,
            "Mavvrik FOCUS destination: got metricsMarker"
        );
        Ok(parsed.metrics_marker)
    }
}

impl<Http: Transport> FocusDestination for FocusMavvrikDestination<Http> {
    type Error = MavvrikDeliveryError<Http::Error>;

    async fn deliver(
        &self,
        content: &[u8],
        time_window: &FocusTimeWindow,
        filename: &str,
    ) -> Result<(), Self::Error> {
        let date = time_window.start_time.format("%Y-%m-%d").to_string();
        let date_epoch = time_window.start_time.timestamp();

        if !self.registered.load(Ordering::SeqCst) {
            self.get_metrics_marker().await?;
        }
        if content.is_empty() {
            tracing::debug!(%date, "Mavvrik FOCUS destination: empty content, advancing marker");
            return update_metrics_marker(self, date_epoch).await;
        }
        tracing::debug!(%date, %filename, bytes = content.len(), "Mavvrik FOCUS destination: uploading");
        let upload_url = signed_url(self, &date).await?;
        upload_to_gcs(self, &upload_url, content).await?;
        update_metrics_marker(self, date_epoch).await
    }
}
