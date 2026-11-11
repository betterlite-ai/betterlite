use betterlite_prompts::arguments::json::Json;
use http::HeaderMap;
use secrecy::SecretString;

use super::error::SapChatError;
use crate::sap_auth::credentials::{Credentials, fetch_credentials, validate_credentials};
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
    // LiteLLM's run_env_setup: a truthy api_key replaces the config's credentials with the service key's
    pub fn setup(
        api_key: Option<&Json>,
        environment: &dyn SapEnvironment,
    ) -> Result<Option<Credentials>, SapChatError> {
        let Some(service_key) = api_key.filter(|key| key.is_truthy()) else {
            return Ok(None);
        };
        let credentials = fetch_credentials(environment, Some(service_key), None)
            .map_err(SapChatError::credentials)?;
        validate_credentials(&credentials).map_err(SapChatError::credentials)?;
        Ok(Some(credentials))
    }

    // LiteLLM's headers and get_complete_url, over the config the caller resolved
    pub fn prepare(&self) -> Result<SapChatCall, SapChatError> {
        Ok(SapChatCall {
            url: format!("{}{COMPLETION_PATH}", self.deployment_url),
            headers: orchestration_headers(&self.token, &self.resource_group)?,
        })
    }
}
