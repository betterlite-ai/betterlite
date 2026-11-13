use std::collections::BTreeMap;

use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};

use super::error::PillarError;
use super::payload::ProtectPayload;
use super::response::ProtectResponse;
use super::settings::PillarSettings;

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const PROTECT_PATH: &str = "/api/v1/protect";

fn header_map(headers: &BTreeMap<String, String>) -> Result<HeaderMap, PillarError> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|source| PillarError::InvalidHeaderName { source })?;
        let value = HeaderValue::from_str(value)
            .map_err(|source| PillarError::InvalidHeaderValue { source })?;
        map.insert(name, value);
    }
    if let Some(secret) = map.get_mut(AUTHORIZATION) {
        secret.set_sensitive(true);
    }
    Ok(map)
}

async fn read_capped_body(mut response: reqwest::Response) -> Result<Vec<u8>, PillarError> {
    let limit = u64::try_from(MAX_RESPONSE_BYTES).unwrap_or(u64::MAX);
    if response
        .content_length()
        .is_some_and(|length| length > limit)
    {
        return Err(PillarError::ResponseTooLarge {
            limit: MAX_RESPONSE_BYTES,
        });
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|source| PillarError::Request { source })?
    {
        body.extend_from_slice(&chunk);
        if body.len() > MAX_RESPONSE_BYTES {
            return Err(PillarError::ResponseTooLarge {
                limit: MAX_RESPONSE_BYTES,
            });
        }
    }
    Ok(body)
}

pub fn parse_protect_response(body: &[u8]) -> Result<ProtectResponse, PillarError> {
    serde_json::from_slice(body).map_err(|source| PillarError::Response { source })
}

pub async fn call_pillar_api(
    client: &reqwest::Client,
    settings: &PillarSettings,
    headers: &BTreeMap<String, String>,
    payload: &ProtectPayload,
) -> Result<ProtectResponse, PillarError> {
    let url = format!("{}{PROTECT_PATH}", settings.api_base);
    let response = client
        .post(url)
        .headers(header_map(headers)?)
        .json(payload)
        .timeout(settings.timeout)
        .send()
        .await
        .map_err(|source| PillarError::Request { source })?;
    let status = response.status();
    if !status.is_success() {
        return Err(PillarError::Status {
            status: status.as_u16(),
        });
    }
    let body = read_capped_body(response).await?;
    parse_protect_response(&body)
}
