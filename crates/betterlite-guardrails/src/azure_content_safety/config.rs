use secrecy::SecretString;
use serde::Deserialize;

use super::api_version::DEFAULT_API_VERSION;

#[derive(Deserialize)]
struct RawConfig {
    api_key: Option<SecretString>,
    api_base: Option<String>,
    api_version: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(from = "RawConfig")]
pub struct AzureContentSafetyConfigModel {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub api_version: String,
}

impl From<RawConfig> for AzureContentSafetyConfigModel {
    fn from(raw: RawConfig) -> Self {
        Self {
            api_key: raw.api_key,
            api_base: raw.api_base,
            api_version: raw.api_version.unwrap_or_else(default_api_version),
        }
    }
}

fn default_api_version() -> String {
    DEFAULT_API_VERSION.to_owned()
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
