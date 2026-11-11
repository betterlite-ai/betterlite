use betterlite_prompts::arguments::json::Json;
use http::HeaderMap;
use secrecy::SecretString;

use super::error::SapChatError;
use crate::sap_auth::credentials::{fetch_credentials, validate_credentials};
use crate::sap_auth::deployment::orchestration_headers;
use crate::sap_auth::environment::SapEnvironment;

pub const COMPLETION_PATH: &str = "/v2/completion";

pub struct SapChatConfig {
    pub token: SecretString,
    pub resource_group: String,
    pub deployment_url: String,
}

#[derive(Debug)]
pub struct SapChatCall {
    pub url: String,
    pub headers: HeaderMap,
}

impl SapChatConfig {
    // LiteLLM's validate_environment and get_complete_url, which the completion path runs back to back
    pub fn prepare(
        &self,
        api_key: Option<&Json>,
        environment: &dyn SapEnvironment,
    ) -> Result<SapChatCall, SapChatError> {
        if let Some(service_key) = api_key.filter(|key| key.is_truthy()) {
            fetch_credentials(environment, Some(service_key), None)
                .and_then(|credentials| validate_credentials(&credentials))
                .map_err(SapChatError::credentials)?;
        }
        Ok(SapChatCall {
            url: format!("{}{COMPLETION_PATH}", self.deployment_url),
            headers: orchestration_headers(&self.token, &self.resource_group)?,
        })
    }
}
