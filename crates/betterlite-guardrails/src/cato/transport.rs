use reqwest::header::HeaderMap;

use super::error::CatoError;
use super::wire::{AnalyzeRequest, AnalyzeResponse};
use super::{CatoGuardrail, MAX_RESPONSE_BYTES};

impl CatoGuardrail {
    pub(super) async fn post_analyze(
        &self,
        request: &AnalyzeRequest,
        headers: HeaderMap,
    ) -> Result<AnalyzeResponse, CatoError> {
        let url = format!("{}/fw/v1/analyze", self.api_base);
        let mut response = self
            .client
            .post(url)
            .headers(headers)
            .json(request)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(|source| {
                if source.is_timeout() {
                    CatoError::Timeout { source }
                } else {
                    CatoError::Request { source }
                }
            })?;
        let status = response.status();
        if status.is_client_error() || status.is_server_error() {
            return Err(CatoError::HttpStatus { status });
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|source| CatoError::Request { source })?
        {
            if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(CatoError::ResponseTooLarge {
                    limit: MAX_RESPONSE_BYTES,
                });
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body).map_err(|source| CatoError::ResponseJson { source })
    }
}
