use secrecy::SecretString;
use serde::Deserialize;

use super::api_version::resolve_api_version;

#[derive(Deserialize)]
struct RawConfig {
    #[serde(rename = "api_key")]
    key: Option<SecretString>,
    #[serde(rename = "api_base")]
    base: Option<String>,
    #[serde(rename = "api_version")]
    version: Option<String>,
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
            api_key: raw.key,
            api_base: raw.base,
            api_version: resolve_api_version(raw.version.as_deref()).to_owned(),
        }
    }
}

fn default_api_version() -> String {
    resolve_api_version(None).to_owned()
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
