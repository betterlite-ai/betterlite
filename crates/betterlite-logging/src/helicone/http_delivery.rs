use std::time::Duration;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::{JsonPost, ProxiedClient, UpstreamResponse};
use secrecy::SecretString;
use thiserror::Error;

use super::envelope::Assembled;

const RESPONSE_LIMIT_BYTES: usize = 1 << 20;

#[derive(Debug, Error)]
pub enum DeliveryError {
    #[error("the Helicone payload could not be written as JSON")]
    Encode(#[source] serde_json::Error),
    #[error("the Helicone request failed")]
    Post(#[source] HttpError),
}

// The pooled client sends Authorization: Bearer <key> and Content-Type: application/json.
pub async fn post(
    client: &ProxiedClient,
    key: &SecretString,
    assembled: Assembled,
    timeout: Duration,
) -> Result<UpstreamResponse, DeliveryError> {
    let body = serde_json::to_vec(&assembled.envelope).map_err(DeliveryError::Encode)?;
    client
        .post_json(JsonPost {
            url: assembled.log_url,
            bearer: key,
            headers: Vec::new(),
            body,
            timeout,
            body_limit: RESPONSE_LIMIT_BYTES,
        })
        .await
        .map_err(DeliveryError::Post)
}

mod tests;
