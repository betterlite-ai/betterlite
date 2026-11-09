use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::Resolve;
use http::HeaderMap;
use http::header::CONTENT_TYPE;
use tokio::time::timeout;

use crate::bedrock_images::error::BedrockImageError;
use crate::bedrock_images::parse::parse_content_type;
use crate::media_fetch::response::{BodyReadError, read_limited_body};
use crate::media_metadata::url_mime::infer_content_type;

const CONNECT_SECONDS: u64 = 5;
const REQUEST_SECONDS: u64 = 600;

// LiteLLM's _DEFAULT_TIMEOUT: 600 seconds for the whole request and every read, 5 seconds to connect
pub const TRANSPORT_LIMITS: TransportLimits = TransportLimits {
    connect_timeout: Duration::from_secs(CONNECT_SECONDS),
    read_timeout: Duration::from_secs(REQUEST_SECONDS),
    body_bytes: TransportLimits::DEFAULT.body_bytes,
    header_bytes: TransportLimits::DEFAULT.header_bytes,
};

pub struct RemoteImage<R> {
    pub client: ProxiedClient,
    pub urls: UserUrlFetcher<R>,
    // the total bound LiteLLM lacks; the Bedrock caller and the helper pass TRANSPORT_LIMITS.read_timeout (600 seconds)
    pub deadline: Duration,
    pub max_download_size_mb: f64,
}

impl<R> RemoteImage<R>
where
    R: Resolve,
{
    // get_image_details and get_image_details_async: one GET through safe_get, then the response processing
    pub async fn image_details(
        &self,
        image_url: &str,
    ) -> Result<(String, String), BedrockImageError> {
        match timeout(self.deadline, self.fetch(image_url)).await {
            Ok(result) => result,
            Err(_elapsed) => Err(BedrockImageError::Deadline),
        }
    }

    async fn fetch(&self, image_url: &str) -> Result<(String, String), BedrockImageError> {
        let fetched = self
            .urls
            .fetch(&self.client, image_url, &HeaderMap::new())
            .await?;
        let status = fetched.response.status();
        if !status.is_success() {
            let final_url = fetched.response.url().to_string();
            return Err(status_error(status.as_u16(), &final_url));
        }
        let headers = fetched.response.headers().clone();
        let body = read_limited_body(fetched.body_stream(), image_url, self.max_download_size_mb)
            .await
            .map_err(|error| match error {
                // reqwest applies the read timeout to the request itself, so its clock can fire before the total deadline
                BodyReadError::Read(HttpError::Request(source)) if source.is_timeout() => {
                    BedrockImageError::Deadline
                }
                BodyReadError::Read(HttpError::Timeout) => BedrockImageError::Deadline,
                other @ (BodyReadError::TooLarge(_)
                | BodyReadError::UncountableLimit { .. }
                | BodyReadError::Read(_)
                | BodyReadError::UncountableBody) => BedrockImageError::Body(other),
            })?;
        post_call_image_processing(image_url, &headers, &body)
    }
}

// _post_call_image_processing: the header, URL and magic-byte type, normalized, with the body in base64
pub fn post_call_image_processing(
    image_url: &str,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<(String, String), BedrockImageError> {
    let declared = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());
    let media_type = infer_content_type(image_url, body, declared)?;
    Ok((BASE64.encode(body), parse_content_type(&media_type)))
}

fn status_error(code: u16, url: &str) -> BedrockImageError {
    let kind = match code {
        100..=199 => "Informational response",
        300..=399 => "Redirect response",
        400..=499 => "Client error",
        500..=599 => "Server error",
        _ => "Invalid status code",
    };
    let reason = http::StatusCode::from_u16(code)
        .ok()
        .and_then(|status| status.canonical_reason())
        .unwrap_or_default();
    BedrockImageError::HttpStatus {
        status: code,
        kind,
        reason,
        url: url.to_owned(),
    }
}
