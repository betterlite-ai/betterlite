use betterlite_http::proxy::limits::{TransportLimits, read_bounded};
use reqwest::header::HeaderMap;

use crate::headroom::text::lossy_text;
use crate::primitives::json_value::JsonValue;

use super::error::HiddenlayerError;

// caller headers carry the provider and project headers; the Authorization header is the auth's own
pub struct DetectionRequest<'request> {
    pub path: &'request str,
    pub headers: HeaderMap,
    pub body: &'request JsonValue,
}

#[derive(Debug)]
pub struct DetectionResponse {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

// httpx's raise_for_status turns any 4xx or 5xx into an error; 3xx responses are answers
pub(crate) async fn settle(
    mut response: reqwest::Response,
) -> Result<DetectionResponse, HiddenlayerError> {
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let body = read_bounded(&mut response, TransportLimits::DEFAULT.body_bytes)
        .await
        .map_err(|source| HiddenlayerError::Body { source })?;
    if status >= 400 {
        return Err(HiddenlayerError::Status {
            status,
            body: lossy_text(&body),
        });
    }
    Ok(DetectionResponse {
        status,
        headers,
        body,
    })
}
