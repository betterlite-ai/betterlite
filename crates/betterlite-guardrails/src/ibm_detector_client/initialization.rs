use betterlite_config::sections::ibm_guardrails::Configured;
use secrecy::ExposeSecret as _;
use thiserror::Error;

use super::config::IbmDetectorConfig;

mod tests;

#[derive(Debug, Error)]
pub enum InitializationError {
    #[error(
        "the IBM Guardrails auth token is required; set IBM_GUARDRAILS_AUTH_TOKEN or pass auth_token"
    )]
    MissingAuthToken,
    #[error("the IBM Guardrails base_url is required")]
    MissingBaseUrl,
    #[error("the IBM Guardrails detector_id is required")]
    MissingDetectorId,
}

impl TryFrom<Configured> for IbmDetectorConfig {
    type Error = InitializationError;

    fn try_from(settings: Configured) -> Result<Self, Self::Error> {
        let auth_token = settings
            .auth_token
            .filter(|token| !token.expose_secret().is_empty())
            .ok_or(InitializationError::MissingAuthToken)?;
        let base_url = settings
            .base_url
            .filter(|url| !url.is_empty())
            .ok_or(InitializationError::MissingBaseUrl)?;
        let detector_id = settings
            .detector_id
            .filter(|id| !id.is_empty())
            .ok_or(InitializationError::MissingDetectorId)?;
        let api_url = if settings.is_detector_server {
            format!("{base_url}/api/v1/text/contents")
        } else {
            format!("{base_url}/api/v2/text/detection/content")
        };
        Ok(Self {
            guardrail_name: settings.guardrail_name,
            auth_token,
            base_url,
            api_url,
            detector_id,
            is_detector_server: settings.is_detector_server,
            detector_params: settings.detector_params,
            extra_headers: settings.extra_headers,
            score_threshold: settings.score_threshold,
            block_on_detection: settings.block_on_detection,
            ssl_verification: settings.ssl_verification,
        })
    }
}
