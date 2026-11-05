use std::sync::PoisonError;

use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use super::GalileoDelivery;
use super::error::DeliveryError;
use super::headers::{Header, json_headers, plain_headers};
use super::transport::Transport;

#[derive(Deserialize)]
struct LoginBody {
    access_token: SecretString,
}

// GalileoObserve._ensure_headers: the cached headers, else the API key header, else a token from POST /login.
pub fn ensure_headers(delivery: &GalileoDelivery, transport: &dyn Transport) -> bool {
    let cached = delivery
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .headers
        .is_some();
    if cached {
        return true;
    }
    if delivery.config.use_v2_api {
        let Some(api_key) = &delivery.config.api_key else {
            return false;
        };
        delivery
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .headers = Some(json_headers("Galileo-API-Key", api_key.clone()));
        return true;
    }
    match login(delivery, transport) {
        Ok(headers) => {
            delivery
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .headers = Some(headers);
            true
        }
        Err(error) => {
            tracing::debug!("Galileo Logger: failed to authenticate: {error}");
            false
        }
    }
}

// GalileoObserve.async_set_galileo_headers.
fn login(
    delivery: &GalileoDelivery,
    transport: &dyn Transport,
) -> Result<Vec<Header>, DeliveryError> {
    let config = &delivery.config;
    let (Some(username), Some(password), Some(base_url)) =
        (&config.username, &config.password, &config.base_url)
    else {
        return Err(DeliveryError::NotConfigured);
    };
    let response = transport.post_form(
        &format!("{base_url}/login"),
        &plain_headers(&[
            ("accept", "application/json"),
            ("Content-Type", "application/x-www-form-urlencoded"),
        ]),
        &[
            ("username", username.as_str()),
            ("password", password.expose_secret()),
        ],
    )?;
    if !(200..300).contains(&response.status) {
        return Err(DeliveryError::LoginStatus {
            status: response.status,
        });
    }
    let body: LoginBody = serde_json::from_str(&response.body).map_err(DeliveryError::LoginBody)?;
    let bearer = format!("Bearer {}", body.access_token.expose_secret());
    Ok(json_headers("Authorization", SecretString::from(bearer)))
}
