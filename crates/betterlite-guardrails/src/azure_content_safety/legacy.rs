use std::collections::BTreeMap;

use betterlite_http::proxy::limits::{ReadError, read_bounded};
use reqwest::header::{CONTENT_TYPE, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};

use super::base::DEFAULT_TIMEOUT;
use super::error::LegacyAzureContentSafetyError;
use super::legacy_thresholds::{SeverityThresholds, configure_thresholds};
use super::legacy_wire::{
    AnalyzeTextOutputType, AnalyzeTextRequest, AnalyzeTextResponse, TextCategory,
};

// azure-ai-contentsafety 1.0.0 chooses its own version for the legacy path, which the hook keeps.
const LEGACY_API_VERSION: &str = "2023-10-01";
const ANALYZE_TEXT_PATH: &str = "text:analyze";
const SUBSCRIPTION_KEY: &str = "ocp-apim-subscription-key";
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Debug)]
pub struct LegacyAzureContentSafetyParams {
    pub endpoint: String,
    pub api_key: SecretString,
    pub thresholds: Option<BTreeMap<String, i64>>,
}

#[derive(Debug)]
pub struct LegacyAzureContentSafety {
    pub endpoint: String,
    pub api_key: SecretString,
    pub client: reqwest::Client,
    pub thresholds: SeverityThresholds,
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
    ) -> Result<Self, LegacyAzureContentSafetyError> {
        let thresholds = configure_thresholds(params.thresholds)?;
        Ok(Self {
            endpoint: params.endpoint,
            api_key: params.api_key,
            client,
            thresholds,
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
        source: Option<&str>,
    ) -> Result<(), LegacyAzureContentSafetyError> {
        tracing::debug!("Testing Azure Content-Safety for: {content}");
        let response = match self.analyze_text(content).await {
            Ok(response) => response,
            Err(LegacyAzureContentSafetyError::HttpStatus { status }) => {
                tracing::debug!(%status, "Error in Azure Content-Safety");
                return Err(LegacyAzureContentSafetyError::HttpStatus { status });
            }
            Err(error) => return Err(error),
        };
        let result = self.compute_result(&response);
        tracing::debug!(?result, "Azure Content-Safety Result");
        for (category, verdict) in result {
            if verdict.filtered {
                return Err(LegacyAzureContentSafetyError::Violation {
                    source: source.map(str::to_owned),
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
    ) -> Result<AnalyzeTextResponse, LegacyAzureContentSafetyError> {
        let url = format!(
            "{}/contentsafety/{ANALYZE_TEXT_PATH}?api-version={LEGACY_API_VERSION}",
            self.endpoint
        );
        let mut subscription_key = HeaderValue::from_str(self.api_key.expose_secret())
            .map_err(|source| LegacyAzureContentSafetyError::InvalidApiKey { source })?;
        subscription_key.set_sensitive(true);
        let mut response = self
            .client
            .post(url)
            .header(HeaderName::from_static(SUBSCRIPTION_KEY), subscription_key)
            .header(CONTENT_TYPE, "application/json")
            .json(&AnalyzeTextRequest {
                text,
                output_type: AnalyzeTextOutputType::EightSeverityLevels,
            })
            .timeout(DEFAULT_TIMEOUT)
            .send()
            .await
            .map_err(request_error)?;
        let status = response.status();
        if status != reqwest::StatusCode::OK {
            return Err(LegacyAzureContentSafetyError::HttpStatus { status });
        }
        let body = read_bounded(&mut response, MAX_RESPONSE_BYTES)
            .await
            .map_err(read_error)?;
        serde_json::from_slice(&body)
            .map_err(|source| LegacyAzureContentSafetyError::ResponseJson { source })
    }
}

fn request_error(source: reqwest::Error) -> LegacyAzureContentSafetyError {
    if source.is_timeout() {
        LegacyAzureContentSafetyError::Timeout { source }
    } else {
        LegacyAzureContentSafetyError::Request { source }
    }
}

fn read_error(error: ReadError) -> LegacyAzureContentSafetyError {
    match error {
        ReadError::TooLarge { limit } => LegacyAzureContentSafetyError::ResponseTooLarge { limit },
        ReadError::Failed(source) => LegacyAzureContentSafetyError::Request { source },
    }
}
