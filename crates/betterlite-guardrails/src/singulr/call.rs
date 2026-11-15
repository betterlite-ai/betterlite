use betterlite_http::proxy::limits::{ReadError, read_bounded};
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use secrecy::ExposeSecret;

use super::error::SingulrError;
use super::payload::SingulrPayload;
use super::settings::SingulrSettings;
use super::wire::Answer;
use super::{GUARD_ENDPOINT, MAX_RESPONSE_BYTES};

const TOKEN_HEADER: &str = "x-singulr-gateway-token";
const APPLICATION_HEADER: &str = "x-singulr-enforcement-entity-id";
const GUARDRAIL_HEADER: &str = "x-singulr-guardrail-id";

pub(super) async fn send(
    client: &reqwest::Client,
    settings: &SingulrSettings,
    payload: &SingulrPayload,
) -> Result<Answer, SingulrError> {
    let endpoint = format!("{}{GUARD_ENDPOINT}", settings.api_base);
    let mut response = client
        .post(endpoint)
        .headers(headers(settings)?)
        .json(payload)
        .timeout(settings.timeout)
        .send()
        .await
        .map_err(|source| SingulrError::Transport { source })?;
    let status = response.status();
    let body = read_bounded(&mut response, MAX_RESPONSE_BYTES)
        .await
        .map_err(read_error)?;
    if !status.is_success() {
        let text = str::from_utf8(&body).map_err(|source| SingulrError::HttpBodyEncoding {
            status: status.as_u16(),
            source,
        })?;
        return Err(SingulrError::HttpStatus {
            status: status.as_u16(),
            body: text.to_owned(),
        });
    }
    serde_json::from_slice(&body).map_err(|source| SingulrError::InvalidResponse { source })
}

fn read_error(source: ReadError) -> SingulrError {
    match source {
        ReadError::TooLarge { limit } => SingulrError::ResponseTooLarge { limit },
        ReadError::Failed(inner) => SingulrError::Transport { source: inner },
    }
}

fn headers(settings: &SingulrSettings) -> Result<HeaderMap, SingulrError> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let token = settings.api_key.as_ref().map(ExposeSecret::expose_secret);
    let optional = [
        (TOKEN_HEADER, token, true),
        (
            APPLICATION_HEADER,
            settings.application_id.as_deref(),
            false,
        ),
        (GUARDRAIL_HEADER, settings.guardrail_id.as_deref(), false),
    ];
    for (name, value, sensitive) in optional {
        if let Some(text) = value.filter(|text| !text.is_empty()) {
            let mut header = HeaderValue::from_str(text)
                .map_err(|source| SingulrError::InvalidHeader { name, source })?;
            header.set_sensitive(sensitive);
            headers.insert(
                HeaderName::try_from(name)
                    .map_err(|source| SingulrError::InvalidHeaderName { name, source })?,
                header,
            );
        }
    }
    Ok(headers)
}
