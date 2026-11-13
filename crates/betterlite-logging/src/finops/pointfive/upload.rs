pub mod sleeper;
pub mod transport;
pub mod url_check;

use std::fmt::Display;
use std::sync::Arc;
use std::time::Duration;

use http::Method;
use http::header::{AUTHORIZATION, CONTENT_ENCODING, CONTENT_TYPE, HOST, HeaderMap, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use url::Url;

use self::sleeper::UploadSleeper;
use self::transport::{UploadHttp, UploadRequest};
use self::url_check::UploadUrlCheck;
use super::types::{PointFiveUploadFailure, PointFiveUploadTarget, RETRYABLE_UPLOAD_STATUS_CODES};

const UPLOAD_KIND: &str = "LITELLM";
const UPLOAD_URL_PATH: &str = "/upload-url";
const PING_PATH: &str = "/ping";
const NDJSON_CONTENT_TYPE: &str = "application/x-ndjson";
const GZIP_ENCODING: &str = "gzip";

#[derive(Serialize)]
struct PresignRequest {
    kind: &'static str,
    #[serde(rename = "byteCount")]
    byte_count: usize,
}

#[derive(Serialize)]
struct PingRequest {
    kind: &'static str,
}

#[derive(Deserialize)]
struct TargetPayload {
    #[serde(rename = "uploadUrl")]
    upload_url: String,
    #[serde(rename = "objectKey")]
    object_key: String,
}

#[derive(Deserialize)]
struct ErrorPayload {
    #[serde(default)]
    error: String,
}

#[derive(Clone)]
pub struct PointFiveUploadClient {
    pub api_key: SecretString,
    pub api_url: String,
    pub http: Arc<dyn UploadHttp>,
    pub url_check: Arc<dyn UploadUrlCheck>,
    pub sleeper: Arc<dyn UploadSleeper>,
    pub max_retries: usize,
    pub request_timeout: Duration,
}

impl PointFiveUploadClient {
    pub async fn upload(&self, body: &[u8]) -> Result<String, PointFiveUploadFailure> {
        for attempt in 0..self.max_retries {
            let failure = match self.upload_once(body).await {
                Ok(object_key) => return Ok(object_key),
                Err(failure) => failure,
            };
            if !failure.retryable {
                return Err(failure);
            }
            if attempt.saturating_add(1) >= self.max_retries {
                return Err(PointFiveUploadFailure {
                    detail: format!(
                        "{}, gave up after {} attempts",
                        failure.detail, self.max_retries
                    ),
                    retryable: true,
                });
            }
            let shift = u32::try_from(attempt).unwrap_or(u32::MAX);
            let delay = Duration::from_secs(1_u64.checked_shl(shift).unwrap_or(u64::MAX));
            self.sleeper.sleep(delay).await;
        }
        Err(PointFiveUploadFailure {
            detail: "max_upload_retries must be at least 1".to_owned(),
            retryable: false,
        })
    }

    pub async fn ping(&self) -> Result<(), PointFiveUploadFailure> {
        let body = encode_request(&PingRequest { kind: UPLOAD_KIND })?;
        self.post(PING_PATH, body).await?;
        Ok(())
    }

    async fn upload_once(&self, body: &[u8]) -> Result<String, PointFiveUploadFailure> {
        let target = self.presign(body.len()).await?;
        self.put(&target, body).await?;
        tracing::debug!(
            bytes = body.len(),
            object_key = %target.object_key,
            "pointfive: uploaded a gzipped batch"
        );
        Ok(target.object_key)
    }

    async fn presign(
        &self,
        byte_count: usize,
    ) -> Result<PointFiveUploadTarget, PointFiveUploadFailure> {
        let request = PresignRequest {
            kind: UPLOAD_KIND,
            byte_count,
        };
        let body = encode_request(&request)?;
        let response = self.post(UPLOAD_URL_PATH, body).await?;
        match serde_json::from_slice::<TargetPayload>(&response) {
            Ok(target) => Ok(PointFiveUploadTarget {
                upload_url: target.upload_url,
                object_key: target.object_key,
            }),
            Err(_ignored) => Err(PointFiveUploadFailure {
                detail: "pointfive api returned an unreadable body".to_owned(),
                retryable: false,
            }),
        }
    }

    async fn post(&self, path: &str, body: Vec<u8>) -> Result<Vec<u8>, PointFiveUploadFailure> {
        let api_url = self.api_url.trim_end_matches('/');
        let url = Url::parse(&format!("{api_url}{path}"))
            .map_err(|source| unreachable("pointfive api", &source))?;
        let bearer = format!("Bearer {}", self.api_key.expose_secret());
        let authorization = HeaderValue::try_from(bearer)
            .map_err(|source| unreachable("pointfive api", &source))?;
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, authorization);
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let request = UploadRequest {
            method: Method::POST,
            url,
            headers,
            body,
            timeout: self.request_timeout,
            follow_redirects: true,
        };
        let response = self
            .http
            .send(request)
            .await
            .map_err(|source| unreachable("pointfive api", &source))?;
        if (200..300).contains(&response.status) {
            return Ok(response.body);
        }
        Err(failure_for(
            "pointfive api",
            response.status,
            &response.body,
        ))
    }

    async fn put(
        &self,
        target: &PointFiveUploadTarget,
        body: &[u8],
    ) -> Result<(), PointFiveUploadFailure> {
        let validated = self
            .url_check
            .check(&target.upload_url)
            .await
            .map_err(|source| PointFiveUploadFailure {
                detail: format!("presigned upload url refused: {source}"),
                retryable: false,
            })?;
        let url = Url::parse(&validated.url)
            .map_err(|source| unreachable("presigned upload", &source))?;
        let headers = upload_headers(validated.host_header)?;
        let request = UploadRequest {
            method: Method::PUT,
            url,
            headers,
            body: body.to_vec(),
            timeout: self.request_timeout,
            follow_redirects: false,
        };
        let response = self
            .http
            .send(request)
            .await
            .map_err(|source| unreachable("presigned upload", &source))?;
        match response.status {
            200..=299 => Ok(()),
            300..=399 => Err(PointFiveUploadFailure {
                detail: format!(
                    "presigned upload redirected with {}, refusing to follow",
                    response.status
                ),
                retryable: false,
            }),
            status => Err(failure_for("presigned upload", status, &response.body)),
        }
    }
}

fn upload_headers(host_header: Option<String>) -> Result<HeaderMap, PointFiveUploadFailure> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(NDJSON_CONTENT_TYPE));
    headers.insert(CONTENT_ENCODING, HeaderValue::from_static(GZIP_ENCODING));
    if let Some(host) = host_header {
        let value = HeaderValue::try_from(host)
            .map_err(|source| unreachable("presigned upload", &source))?;
        headers.insert(HOST, value);
    }
    Ok(headers)
}

fn unreachable<Cause>(what: &str, cause: &Cause) -> PointFiveUploadFailure
where
    Cause: Display,
{
    PointFiveUploadFailure {
        detail: format!("{what} unreachable: {cause}"),
        retryable: true,
    }
}

fn encode_request<Body>(body: &Body) -> Result<Vec<u8>, PointFiveUploadFailure>
where
    Body: Serialize,
{
    serde_json::to_vec(body).map_err(|source| PointFiveUploadFailure {
        detail: format!("pointfive api request could not be encoded: {source}"),
        retryable: false,
    })
}

fn failure_for(what: &str, status: u16, body: &[u8]) -> PointFiveUploadFailure {
    let detail = format!("{what} returned {status}");
    let reason = serde_json::from_slice::<ErrorPayload>(body)
        .map_or_else(|_ignored| String::new(), |payload| payload.error);
    PointFiveUploadFailure {
        detail: if reason.is_empty() {
            detail
        } else {
            format!("{detail}, {reason}")
        },
        retryable: RETRYABLE_UPLOAD_STATUS_CODES.contains(&status),
    }
}
