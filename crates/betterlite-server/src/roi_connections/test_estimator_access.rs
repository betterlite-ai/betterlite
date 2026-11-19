use axum::http::Method;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_roi::settings::ROISettings;
use secrecy::SecretString;
use serde::Deserialize;

use crate::roi_github::transport::Transport;

use super::error::RoiConnectionsError;
use super::gateway_http_client::GatewayHttpClient;
use super::gateway_key::gateway_key;

pub const MODELS_PATH: &str = "/v1/models";
pub const CANNOT_ACCESS: &str = "The estimator key cannot access the selected model.";
pub const CANNOT_CONNECT: &str = "The estimator key could not connect to the gateway.";

#[derive(Deserialize)]
struct GatewayModels {
    data: Vec<GatewayModel>,
}

#[derive(Deserialize)]
struct GatewayModel {
    id: String,
}

pub(super) async fn test_estimator_access<T>(
    settings: &ROISettings,
    master_key: Option<&SecretString>,
    client: &GatewayHttpClient<T>,
) -> Result<(), RoiConnectionsError>
where
    T: Transport,
{
    let credential = gateway_key(settings, master_key)?;
    let response = client
        .request(Method::GET, MODELS_PATH, &credential, RequestBody::Empty)
        .await
        .map_err(|_error| RoiConnectionsError::Conflict(CANNOT_CONNECT))?;
    let models: GatewayModels = serde_json::from_slice(&response.body)
        .map_err(|_error| RoiConnectionsError::Conflict(CANNOT_CONNECT))?;
    if models
        .data
        .iter()
        .any(|model| model.id == settings.estimator_model)
    {
        return Ok(());
    }
    Err(RoiConnectionsError::Conflict(CANNOT_ACCESS))
}

mod tests;
