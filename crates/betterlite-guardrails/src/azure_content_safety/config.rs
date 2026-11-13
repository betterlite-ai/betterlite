use secrecy::SecretString;
use serde::Deserialize;

use super::api_version::resolve_api_version;

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
            api_version: resolve_api_version(raw.api_version.as_deref()).to_owned(),
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
