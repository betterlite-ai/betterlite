use std::sync::atomic::Ordering;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use url::Url;

use super::destination::FocusMavvrikDestination;
use super::error::MavvrikDeliveryError;
use super::transport::{Method, Transport, TransportRequest, TransportResponse};
use super::validation::validate_gcs_url;

const API_TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const API_ERROR_BODY_BYTES: usize = 200;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum MetricsMarker {
    Epoch(i64),
    Text(String),
}

impl Default for MetricsMarker {
    fn default() -> Self {
        Self::Epoch(0)
    }
}

#[derive(Serialize)]
pub(super) struct RegisterRequest<'name> {
    pub(super) name: &'name str,
}

#[derive(Deserialize)]
pub(super) struct RegisterResponse {
    #[serde(rename = "metricsMarker")]
    pub(super) metrics_marker: Option<MetricsMarker>,
}

#[derive(Serialize)]
struct MarkerRequest {
    #[serde(rename = "metricsMarker")]
    metrics_marker: i64,
}

#[derive(Deserialize)]
struct UploadUrlResponse {
    url: Option<String>,
}

pub(super) async fn send_api<Http>(
    destination: &FocusMavvrikDestination<Http>,
    method: Method,
    url: String,
    body: Vec<u8>,
) -> Result<TransportResponse, MavvrikDeliveryError<Http::Error>>
where
    Http: Transport,
{
    let request = TransportRequest {
        method,
        url,
        headers: vec![("Content-Type", "application/json".to_owned())],
        api_key: Some(destination.api_key.clone()),
        body,
        timeout: API_TIMEOUT,
    };
    destination.send(request).await
}

pub(super) fn reject_disconnected<Http>(
    destination: &FocusMavvrikDestination<Http>,
    response: &TransportResponse,
) -> Result<(), MavvrikDeliveryError<Http::Error>>
where
    Http: Transport,
{
    if response.status == 410 {
        destination.registered.store(false, Ordering::SeqCst);
        return Err(MavvrikDeliveryError::Disconnected);
    }
    Ok(())
}

pub(super) async fn signed_url<Http>(
    destination: &FocusMavvrikDestination<Http>,
    date: &str,
) -> Result<String, MavvrikDeliveryError<Http::Error>>
where
    Http: Transport,
{
    let url = Url::parse_with_params(
        &destination.upload_url_endpoint,
        [("name", date), ("type", "metrics"), ("datetime", date)],
    )
    .map_err(MavvrikDeliveryError::RequestUrl)?;
    let response = send_api(destination, Method::Get, String::from(url), Vec::new()).await?;
    if response.status >= 400 {
        return Err(MavvrikDeliveryError::SignedUrlFailed {
            status: response.status,
            body: response.text_prefix(API_ERROR_BODY_BYTES),
        });
    }
    let parsed: UploadUrlResponse =
        serde_json::from_slice(&response.body).map_err(MavvrikDeliveryError::Json)?;
    let signed = parsed
        .url
        .filter(|signed| !signed.is_empty())
        .ok_or(MavvrikDeliveryError::SignedUrlMissing)?;
    validate_gcs_url(&signed, "signed URL")?;
    Ok(signed)
}

pub(super) async fn update_metrics_marker<Http>(
    destination: &FocusMavvrikDestination<Http>,
    date_epoch: i64,
) -> Result<(), MavvrikDeliveryError<Http::Error>>
where
    Http: Transport,
{
    let body = serde_json::to_vec(&MarkerRequest {
        metrics_marker: date_epoch,
    })
    .map_err(MavvrikDeliveryError::Json)?;
    let agent_url = destination.agent_url.clone();
    let response = send_api(destination, Method::Patch, agent_url, body).await?;
    reject_disconnected(destination, &response)?;
    if response.status >= 400 {
        return Err(MavvrikDeliveryError::MarkerUpdateFailed {
            status: response.status,
            body: response.text_prefix(API_ERROR_BODY_BYTES),
        });
    }
    tracing::debug!(
        date_epoch,
        "Mavvrik FOCUS destination: metricsMarker advanced"
    );
    Ok(())
}
