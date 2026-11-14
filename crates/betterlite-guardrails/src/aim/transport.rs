use std::time::Duration;

use async_trait::async_trait;
use reqwest::StatusCode;
use reqwest::header::HeaderMap;

use super::error::AimError;

pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

pub struct AimHttpResponse {
    pub status: StatusCode,
    pub body: Vec<u8>,
}

#[async_trait]
pub trait AimPoster: Send + Sync {
    async fn post(
        &self,
        url: &str,
        headers: &HeaderMap,
        body: &[u8],
        timeout: Option<Duration>,
    ) -> Result<AimHttpResponse, AimError>;
}

pub struct HttpPoster {
    pub client: reqwest::Client,
}

#[async_trait]
impl AimPoster for HttpPoster {
    async fn post(
        &self,
        url: &str,
        headers: &HeaderMap,
        body: &[u8],
        timeout: Option<Duration>,
    ) -> Result<AimHttpResponse, AimError> {
        let mut request = self
            .client
            .post(url)
            .headers(headers.clone())
            .body(body.to_vec());
        if let Some(timeout) = timeout {
            request = request.timeout(timeout);
        }
        let mut response = request.send().await.map_err(request_error)?;
        let status = response.status();
        let mut collected = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(request_error)? {
            if collected.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(AimError::ResponseTooLarge(MAX_RESPONSE_BYTES));
            }
            collected.extend_from_slice(&chunk);
        }
        Ok(AimHttpResponse {
            status,
            body: collected,
        })
    }
}

fn request_error(source: reqwest::Error) -> AimError {
    if source.is_timeout() {
        AimError::Timeout(source)
    } else {
        AimError::Request(source)
    }
}
