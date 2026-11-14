use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use betterlite_http::proxy::limits::read_bounded;
use jiff::Timestamp;
use reqwest::StatusCode;
use reqwest::header::{CONTENT_TYPE, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::time::sleep;

use super::base::{MAX_RESPONSE_BYTES, SUBSCRIPTION_KEY};
use super::error::{AzureContentSafetyError, read_error, request_error};
use super::legacy_retry::{BACKOFF_FACTOR, STATUS_RETRIES, is_retry, retry_delay};
use super::legacy_thresholds::{SeverityThresholds, configure_thresholds};
use super::legacy_wire::{
    AnalyzeTextOutputType, AnalyzeTextRequest, AnalyzeTextResponse, TextCategory,
};

// azure-ai-contentsafety 1.0.0 chooses its own version for the legacy path, which the hook keeps.
const LEGACY_API_VERSION: &str = "2023-10-01";
const ANALYZE_TEXT_PATH: &str = "text:analyze";
// azure-core's aiohttp transport waits 300 seconds to connect and 300 seconds per read.
pub const LEGACY_TIMEOUT: Duration = Duration::from_secs(300);
// An azure-core timeout carries no status_code, so the proxy answers it with 500.
const TIMEOUT_STATUS: StatusCode = StatusCode::INTERNAL_SERVER_ERROR;

#[derive(Debug)]
pub struct LegacyAzureContentSafetyParams {
    pub endpoint: String,
    pub api_key: SecretString,
    pub thresholds: BTreeMap<String, i64>,
}

pub struct LegacyAzureContentSafety {
    pub endpoint: String,
    pub api_key: SecretString,
    pub client: reqwest::Client,
    pub thresholds: SeverityThresholds,
    pub timeout: Duration,
    pub retry_backoff_factor: Duration,
    pub clock: fn() -> Timestamp,
}

// the clock is a fn pointer, so the derive would print an address
impl fmt::Debug for LegacyAzureContentSafety {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LegacyAzureContentSafety")
            .field("endpoint", &self.endpoint)
            .field("api_key", &self.api_key)
            .field("client", &self.client)
            .field("thresholds", &self.thresholds)
            .field("timeout", &self.timeout)
            .field("retry_backoff_factor", &self.retry_backoff_factor)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CategoryVerdict {
    pub filtered: bool,
    pub severity: i64,
}

impl LegacyAzureContentSafety {
    // CustomLogger.enforces_request_content, which makes the proxy run this hook before the call.
    pub const ENFORCES_REQUEST_CONTENT: bool = true;

    pub fn new(
        params: LegacyAzureContentSafetyParams,
        client: reqwest::Client,
    ) -> Result<Self, AzureContentSafetyError> {
        let thresholds = configure_thresholds(params.thresholds)?;
        Ok(Self {
            endpoint: params.endpoint,
            api_key: params.api_key,
            client,
            thresholds,
            timeout: LEGACY_TIMEOUT,
            retry_backoff_factor: BACKOFF_FACTOR,
            clock: Timestamp::now,
        })
    }

    #[must_use]
    pub fn compute_result(
        &self,
        response: &AnalyzeTextResponse,
    ) -> BTreeMap<TextCategory, CategoryVerdict> {
        let mut severities: BTreeMap<TextCategory, i64> = BTreeMap::new();
        for analysis in &response.categories_analysis {
            if let Some(severity) = analysis.severity {
                severities.insert(analysis.category, severity);
            }
        }
        let mut result = BTreeMap::new();
        for category in TextCategory::ALL {
            let Some(severity) = severities.get(&category).copied() else {
                continue;
            };
            result.insert(
                category,
                CategoryVerdict {
                    filtered: severity >= self.thresholds.for_category(category),
                    severity,
                },
            );
        }
        result
    }

    pub async fn test_violation(
        &self,
        content: &str,
        origin: Option<&str>,
    ) -> Result<(), AzureContentSafetyError> {
        tracing::debug!("Testing Azure Content-Safety for: {content}");
        let response = match self.analyze_text(content).await {
            Ok(response) => response,
            Err(error @ AzureContentSafetyError::HttpStatus { .. }) => {
                tracing::debug!(%error, "Error in Azure Content-Safety");
                return Err(error);
            }
            Err(error) => return Err(error),
        };
        let result = self.compute_result(&response);
        tracing::debug!(?result, "Azure Content-Safety Result");
        for (category, verdict) in result {
            if verdict.filtered {
                return Err(AzureContentSafetyError::Violation {
                    origin: origin.map(str::to_owned),
                    category,
                    severity: verdict.severity,
                });
            }
        }
        Ok(())
    }

    async fn analyze_text(
        &self,
        text: &str,
    ) -> Result<AnalyzeTextResponse, AzureContentSafetyError> {
        let mut response = self
            .send_with_retries(&AnalyzeTextRequest {
                text,
                output_type: AnalyzeTextOutputType::EightSeverityLevels,
            })
            .await?;
        let status = response.status();
        let body = read_bounded(&mut response, MAX_RESPONSE_BYTES)
            .await
            .map_err(|error| read_error(error, TIMEOUT_STATUS))?;
        if status != StatusCode::OK {
            return Err(AzureContentSafetyError::HttpStatus {
                status,
                body: String::from_utf8(body).unwrap_or_default(),
            });
        }
        serde_json::from_slice(&body)
            .map_err(|source| AzureContentSafetyError::ResponseJson { source })
    }

    // azure-core sends a retryable answer again up to STATUS_RETRIES times, then hands back the last one.
    async fn send_with_retries(
        &self,
        request: &AnalyzeTextRequest<'_>,
    ) -> Result<reqwest::Response, AzureContentSafetyError> {
        let url = format!(
            "{}/contentsafety/{ANALYZE_TEXT_PATH}?api-version={LEGACY_API_VERSION}",
            self.endpoint
        );
        let mut subscription_key = HeaderValue::from_str(self.api_key.expose_secret())
            .map_err(|source| AzureContentSafetyError::InvalidApiKey { source })?;
        subscription_key.set_sensitive(true);
        let mut retries = 0_u32;
        loop {
            let response = self
                .client
                .post(&url)
                .header(HeaderName::from_static(SUBSCRIPTION_KEY), &subscription_key)
                .header(CONTENT_TYPE, "application/json")
                .json(request)
                .timeout(self.timeout)
                .send()
                .await
                .map_err(|source| request_error(source, TIMEOUT_STATUS))?;
            if retries >= STATUS_RETRIES || !is_retry(response.status(), response.headers()) {
                return Ok(response);
            }
            retries = retries.saturating_add(1);
            let now = (self.clock)();
            sleep(retry_delay(
                response.headers(),
                retries,
                self.retry_backoff_factor,
                now,
            ))
            .await;
        }
    }
}
