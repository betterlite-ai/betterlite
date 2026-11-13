use secrecy::SecretString;
use serde::Deserialize;

use super::api_version::DEFAULT_API_VERSION;

#[derive(Debug, Clone, Deserialize)]
pub struct AzureContentSafetyConfigModel {
    #[serde(default)]
    pub api_key: Option<SecretString>,
    #[serde(default)]
    pub api_base: Option<String>,
    #[serde(default = "default_api_version")]
    pub api_version: Option<String>,
}

fn default_api_version() -> Option<String> {
    Some(DEFAULT_API_VERSION.to_owned())
}

impl Default for AzureContentSafetyConfigModel {
    fn default() -> Self {
        Self {
            api_key: None,
            api_base: None,
            api_version: default_api_version(),
        }
    }
}
