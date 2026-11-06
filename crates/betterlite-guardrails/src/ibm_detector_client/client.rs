use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::tls::TlsPolicy;
use betterlite_http::proxy::base_builder;
use betterlite_http::tls::apply_policy;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use reqwest::{Client, Response};
use secrecy::ExposeSecret as _;
use serde::Serialize;

use super::config::IbmDetectorConfig;
use super::wire::{Detection, DetectorServerRequest, OrchestratorRequest, OrchestratorResponse};
use error::IbmClientError;

pub mod error;
mod tests;

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug, Clone)]
pub struct IbmDetectorClient {
    pub config: IbmDetectorConfig,
    pub http: Client,
    pub timeout: Duration,
}

impl IbmDetectorClient {
    pub fn new(config: IbmDetectorConfig) -> Result<Self, IbmClientError> {
        let policy = TlsPolicy {
            verification: config.ssl_verification.clone(),
            ca_file: None,
            certificate: None,
            http2: false,
            ecdh_curve: None,
            user_agent: format!("betterlite/{}", env!("CARGO_PKG_VERSION")),
        };
        let builder = apply_policy(base_builder(), &policy)?;
        let http = builder.build().map_err(IbmClientError::Build)?;
        Ok(Self {
            config,
            http,
            timeout: DEFAULT_TIMEOUT,
        })
    }

    pub async fn call_detector_server(
        &self,
        contents: &[String],
    ) -> Result<Vec<Vec<Detection>>, IbmClientError> {
        let payload = DetectorServerRequest {
            contents,
            detector_params: &self.config.detector_params,
        };
        let response = self
            .send(
                &self.config.api_url,
                &payload,
                Some(&self.config.detector_id),
            )
            .await?;
        serde_json::from_slice(&response).map_err(IbmClientError::ResponseJson)
    }

    pub async fn call_orchestrator(&self, content: &str) -> Result<Vec<Detection>, IbmClientError> {
        let payload = OrchestratorRequest {
            content,
            detectors: BTreeMap::from([(
                self.config.detector_id.as_str(),
                &self.config.detector_params,
            )]),
        };
        let response = self.send(&self.config.api_url, &payload, None).await?;
        let parsed: OrchestratorResponse =
            serde_json::from_slice(&response).map_err(IbmClientError::ResponseJson)?;
        Ok(parsed.detections)
    }

    pub async fn scan(&self, contents: &[String]) -> Result<Vec<Vec<Detection>>, IbmClientError> {
        if self.config.is_detector_server {
            return self.call_detector_server(contents).await;
        }
        let mut per_text = Vec::with_capacity(contents.len());
        for content in contents {
            per_text.push(self.call_orchestrator(content).await?);
        }
        Ok(per_text)
    }

    pub fn request_headers(&self, detector_id: Option<&str>) -> Result<HeaderMap, IbmClientError> {
        let mut headers = HeaderMap::new();
        let authorization = format!("Bearer {}", self.config.auth_token.expose_secret());
        let authorization_value = HeaderValue::from_str(&authorization).map_err(|source| {
            IbmClientError::HeaderValue {
                name: "Authorization".to_owned(),
                source,
            }
        })?;
        headers.insert(AUTHORIZATION, authorization_value);
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(id) = detector_id {
            let detector_value =
                HeaderValue::from_str(id).map_err(|source| IbmClientError::HeaderValue {
                    name: "detector-id".to_owned(),
                    source,
                })?;
            headers.insert(HeaderName::from_static("detector-id"), detector_value);
        }
        for (name, raw_value) in &self.config.extra_headers {
            let header = HeaderName::from_bytes(name.as_bytes()).map_err(|source| {
                IbmClientError::HeaderName {
                    name: name.clone(),
                    source,
                }
            })?;
            let header_value =
                HeaderValue::from_str(raw_value).map_err(|source| IbmClientError::HeaderValue {
                    name: name.clone(),
                    source,
                })?;
            headers.insert(header, header_value);
        }
        Ok(headers)
    }

    async fn send<P>(
        &self,
        url: &str,
        payload: &P,
        detector_id: Option<&str>,
    ) -> Result<Vec<u8>, IbmClientError>
    where
        P: Serialize + Sync,
    {
        let encoded = serde_json::to_vec(payload).map_err(IbmClientError::Encode)?;
        let response = self
            .http
            .post(url)
            .headers(self.request_headers(detector_id)?)
            .body(encoded)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(IbmClientError::Request)?;
        let status = response.status();
        let body = read_bounded(response).await?;
        if !status.is_success() {
            return Err(IbmClientError::Status { status });
        }
        Ok(body)
    }
}

async fn read_bounded(mut response: Response) -> Result<Vec<u8>, IbmClientError> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(IbmClientError::Read)? {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(IbmClientError::TooLarge {
                limit: MAX_RESPONSE_BYTES,
            });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}
